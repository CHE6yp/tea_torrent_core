use crate::big_endian_to_u32;
use crate::tf;
use crate::BLOCK_SIZE;
use fs::OpenOptions;
use sha1::Digest;
use sha1::Sha1;
use std::collections::HashMap;
use std::fs;
use std::io::stdout;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::sync::mpsc::channel;
use tf::TorrentFile;
use threadpool::ThreadPool;

#[derive(Debug, Clone)]
pub struct Content {
    pieces: HashMap<u32, Piece>,
    files: Vec<tf::File>,
    pub missing_pieces: Vec<usize>,
    available_pieces: Vec<usize>,
    pub destination_path: String,
}

impl Content {
    pub fn new(tf: &TorrentFile) -> Content {
        let mut pieces = HashMap::new();
        for i in 0..tf.info.piece_count - 1 {
            pieces.insert(i, Piece::new(i, tf.info.piece_length, tf));
        }
        //last piece is probably a different size
        pieces.insert(
            tf.info.piece_count - 1,
            Piece::new(tf.info.piece_count - 1, tf.info.get_last_piece_size(), tf),
        );

        Content {
            pieces: pieces,
            files: vec![],
            missing_pieces: vec![],
            available_pieces: vec![],
            destination_path: "downloads".to_string(),
        }
    }

    pub fn preallocate(&self, tf: &TorrentFile) {
        println!("Preallocating files");
        let file_path = format!(
            "{}/{}",
            self.destination_path,
            if tf.info.files.len() > 1 {
                &tf.info.name
            } else {
                ""
            }
        );
        let path = std::path::Path::new(&file_path);
        //TODO ok wtf did i do here?
        let files: Vec<tf::File> = tf
            .info
            .files
            .iter()
            .map(|f| -> tf::File {
                let mut fc = f.clone();
                fc.path = path.join(f.path.clone());
                fc
            })
            .collect();

        for file in files {
            if let Some(dir_path) = file.path.as_path().parent() {
                fs::create_dir_all(dir_path).unwrap();
            }

            //preallocate file
            let f = OpenOptions::new()
                .write(true)
                .create(true)
                .open(file.path)
                .unwrap();
            f.set_len(file.length as u64).unwrap();
        }
        println!("Preallocation complete");
    }

    pub fn check_content_hash(&mut self, tf: &TorrentFile) {
        println!("Checking files hash");
        let mut missing_pieces = vec![];
        let mut available_pieces = vec![];

        let pool = ThreadPool::new(9);

        let (tx, rx) = channel();

        for p in 0..tf.info.piece_count {
            let mut read_buf = Vec::with_capacity(tf.info.piece_length as usize);
            let (offset, files) = tf.info.get_piece_files(p as usize);

            let mut first = true;
            let mut r = 0;
            for file in files {
                let file_path = format!(
                    "{}/{}",
                    self.destination_path,
                    if tf.info.files.len() > 1 {
                        &tf.info.name
                    } else {
                        ""
                    }
                );
                let path = std::path::Path::new(&file_path);
                let mut fc = file.clone();
                fc.path = path.join(file.path.clone());

                let mut f = OpenOptions::new().read(true).open(fc.path).unwrap();

                if first {
                    f.seek(SeekFrom::Start(offset as u64)).expect("seek failed");
                    first = false;
                }

                r += f
                    .take(tf.info.piece_length as u64 - r as u64)
                    .read_to_end(&mut read_buf)
                    .unwrap();
            }
            let tx = tx.clone();

            pool.execute(move || {
                let mut hasher = Sha1::new();

                hasher.update(&read_buf);
                let hexes = hasher.finalize();

                let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");
                tx.send((p, hexes))
                    .expect("channel will be there waiting for the pool");
            });
        }

        rx.iter()
            .take(tf.info.piece_count as usize)
            .for_each(|(p, hexes)| {
                if hexes != tf.info.get_piece_hash(p as usize) {
                    print!("\x1b[91m");
                    missing_pieces.push(p as usize);
                } else {
                    available_pieces.push(p as usize);
                    print!("\x1b[92m");
                }
                print!("{} ", p);
                stdout().flush().unwrap();
            });

        println!("\x1b[0m");
        self.missing_pieces = missing_pieces;
        self.available_pieces = available_pieces;
    }

    pub fn add_block(&mut self, piece: u32, block: Vec<u8>) -> Option<bool> {
        self.pieces.get_mut(&piece).unwrap().add_block(block)

        // self.pieces.remove(&piece);
    }
}

#[derive(Debug, Clone)]
pub struct Piece {
    number: u32,
    offset: usize,
    buf: Vec<u8>,
    hash: [u8; 20],
    files: Vec<tf::File>,
    pub block_count: u32,
    pub block_count_goal: u32,
}

impl Piece {
    pub fn new(number: u32, size: u32, tf: &TorrentFile) -> Piece {
        /*
            TODO smaller blocks are only in the last piece
            maybe i need to get this out of constructor?
        */
        let mut remainder = size % BLOCK_SIZE;
        let mut smaller_blocks = 0;
        while remainder != 0 {
            remainder = remainder & (remainder - 1);
            smaller_blocks += 1;
        }
        let block_count_goal = size / BLOCK_SIZE + smaller_blocks;

        let (offset, files) = tf.info.get_piece_files(number as usize);
        Piece {
            number,
            hash: tf.info.get_piece_hash(number as usize).try_into().unwrap(),
            files: files.to_vec(),
            offset,
            buf: vec![0; size.try_into().unwrap()],
            block_count_goal,
            block_count: 0,
        }
    }

    pub fn add_block(&mut self, block: Vec<u8>) -> Option<bool> {
        self.block_count += 1;
        let offset = big_endian_to_u32(&block[5..9].try_into().unwrap());

        // println!(
        //     "Got piece {} of {{}} ({{}}%), offset {}/{{}} ({{}}%)  from {{}}",
        //     big_endian_to_u32(&block[1..5].try_into().unwrap()),
        //     big_endian_to_u32(&block[5..9].try_into().unwrap()),
        // );

        let new_buf = [
            &self.buf[..offset as usize],
            &block[9..],
            &self.buf[(offset + block[9..].len() as u32) as usize..],
        ]
        .concat();
        self.buf = new_buf;

        if self.block_count == self.block_count_goal {
            return Some(self.write());
        }
        None
    }

    pub fn write(&self) -> bool {
        //if whole piece is downloaded
        let mut buffer = self.buf.clone();
        //check hash
        let mut hasher = Sha1::new();
        hasher.update(&buffer);
        let hexes = hasher.finalize();
        let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");
        if hexes != self.hash {
            return false;
        }

        //==========
        let mut of = self.offset;

        let mut prev_written_bytes = 0;
        for file in &self.files {
            let file_path = format!(
                "downloads/{}",
                // if tf.info.files.len() > 1 {
                //     &tf.info.name
                // } else {
                "" // }
            );
            let path = std::path::Path::new(&file_path);
            let mut fc = file.clone();
            fc.path = path.join(file.path.clone());
            let mut f = OpenOptions::new().write(true).open(fc.path).unwrap();

            f.seek(SeekFrom::Start(of as u64)).expect("seek failed");

            let how_much = std::cmp::min(file.length - of, self.buf.len() - prev_written_bytes);
            let written = f.write(&buffer[..how_much]);
            match written {
                Ok(count) => {
                    prev_written_bytes = count;
                    buffer.drain(..count);
                    of = 0;
                }
                Err(e) => {
                    println!("Write failed\n{:?}", e);
                }
            }
        }

        true
    }
}
