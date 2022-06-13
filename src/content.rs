use crate::tf;
use crate::BLOCK_SIZE;
use fs::OpenOptions;
use sha1::Digest;
use sha1::Sha1;
use std::path::PathBuf;

use std::fs;
use std::io::stdout;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::sync::mpsc::channel;
use std::thread;
use tf::TorrentFile;
// use threadpool::ThreadPool;

#[derive(Debug, Clone)]
pub struct Content {
    pieces: Vec<Piece>,
    files: Vec<(PathBuf, usize)>,
    pub missing_pieces: Vec<usize>,
    available_pieces: Vec<usize>,
    pub destination_path: String,
}

impl Content {
    pub fn new(tf: &TorrentFile) -> Content {
        let file_path = format!(
            "{}/{}",
            "downloads",
            if tf.info.files.len() > 1 {
                &tf.info.name
            } else {
                ""
            }
        );
        //I think im converting tfFile to content File here
        let files: Vec<(PathBuf, usize)> = tf
            .info
            .files
            .iter()
            .map(|f| -> (PathBuf, usize) {
                let path = format!("{}/{}", file_path, f.path);
                (PathBuf::from(path), f.length)
            })
            .collect();

        let mut pieces = vec![];
        for piece_number in 0..tf.info.piece_count - 1 {
            let (offset, piece_files) = Content::get_piece_files(
                piece_number as usize,
                &files,
                tf.info.piece_length as usize,
                tf.info.length,
            );
            let hash = tf
                .info
                .get_piece_hash(piece_number as usize)
                .try_into()
                .unwrap();
            pieces.push(Piece::new(
                piece_number,
                tf.info.piece_length,
                offset,
                piece_files,
                hash,
            ));
        }
        //last piece is probably a different size
        let (offset, piece_files) = Content::get_piece_files(
            (tf.info.piece_count - 1) as usize,
            &files,
            tf.info.piece_length as usize,
            tf.info.length,
        );
        let hash = tf
            .info
            .get_piece_hash((tf.info.piece_count - 1) as usize)
            .try_into()
            .unwrap();
        pieces.push(Piece::new(
            tf.info.piece_count - 1,
            tf.info.get_last_piece_size(),
            offset,
            piece_files,
            hash,
        ));

        Content {
            pieces: pieces,
            files,
            missing_pieces: vec![],
            available_pieces: vec![],
            destination_path: "downloads".to_string(),
        }
    }

    pub fn preallocate(&self) {
        println!("Preallocating files");

        for file in &self.files {
            if let Some(dir_path) = file.0.as_path().parent() {
                fs::create_dir_all(dir_path).unwrap();
            }
            //preallocate file
            let f = OpenOptions::new()
                .write(true)
                .create(true)
                .open(file.0.as_path())
                .unwrap();
            f.set_len(file.1 as u64).unwrap();
        }
        println!("Preallocation complete");
    }

    pub fn check_content_hash(&mut self) {
        println!("Checking files hash");
        let mut missing_pieces = vec![];
        let mut available_pieces = vec![];

        //todo maybe return this threadPool?
        // let pool = ThreadPool::new(9);

        let (tx, rx) = channel();

        for piece in &mut self.pieces {
            let tx = tx.clone();

            // pool.execute(move || {
            //     tx.send((p.lock().unwrap().number, p.lock().unwrap().check_hash(&read_buf)))
            //         .expect("channel will be there waiting for the pool");
            //     // check_hash_and_send(tx, &piece, &read_buf)
            // });
            thread::scope(|s| {
                s.spawn(move || {
                    let mut read_buf = Vec::with_capacity(piece.size as usize);
                    let offset = piece.offset;
                    let files = &piece.files;

                    let mut first = true;
                    let mut r = 0;
                    for file in files {
                        let mut f = OpenOptions::new()
                            .read(true)
                            .open(file.0.as_path())
                            .unwrap();

                        if first {
                            f.seek(SeekFrom::Start(offset as u64)).expect("seek failed");
                            first = false;
                        }

                        r += f
                            .take(piece.size as u64 - r as u64)
                            .read_to_end(&mut read_buf)
                            .unwrap();
                    }
                    tx.send((piece.number, piece.check_hash(&read_buf)))
                        .expect("channel will be there waiting for the pool");
                });
            });
        }

        rx.iter()
            .take(self.pieces.len())
            .for_each(|(piece_number, hash_correct)| {
                if !hash_correct {
                    print!("\x1b[91m");
                    missing_pieces.push(piece_number as usize);
                } else {
                    available_pieces.push(piece_number as usize);
                    print!("\x1b[92m");
                }
                print!("{} ", piece_number);
                stdout().flush().unwrap();
            });

        println!("\x1b[0m");
        self.missing_pieces = missing_pieces;
        self.available_pieces = available_pieces;
    }

    pub fn add_block(&mut self, piece_number: usize, offset: usize, block: &[u8]) -> Option<bool> {
        self.pieces[piece_number].add_block(offset, block)
    }

    pub fn get_piece_files(
        piece: usize,
        files: &Vec<(PathBuf, usize)>,
        piece_length: usize,
        length: usize,
    ) -> (usize, Vec<(PathBuf, usize)>) {
        let mut l = 0;
        let mut first_file = 0;
        let mut last_file = 0;
        for fnum in 0..files.len() {
            l += files[fnum].1;
            if (piece + 1) * piece_length > length {
                last_file = files.len() - 1;
                break;
            }
            if (piece + 1) * piece_length <= l {
                last_file = fnum;
                break;
            }
        }
        l = 0;
        for fnum in 0..files.len() {
            l += files[fnum].1;
            if piece * piece_length <= l {
                first_file = fnum;
                break;
            }
        }

        let first_offset = files[first_file].1 - (l - (piece * piece_length));
        let files = &files[first_file..=last_file];
        (first_offset, files.to_vec())
    }
}

#[derive(Debug, Clone)]
struct Piece {
    number: u32,
    size: u32,
    offset: usize,
    status: PieceStatus,
    hash: [u8; 20],
    //TODO
    //make files a slice, I DARE YOU
    //you will fall into
    //a liftime of lifitime annotations
    //hell
    files: Vec<(PathBuf, usize)>,
    block_count: u32,
    block_count_goal: u32,
}

impl Piece {
    fn new(
        number: u32,
        size: u32,
        offset: usize,
        files: Vec<(PathBuf, usize)>,
        hash: [u8; 20],
    ) -> Piece {
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

        Piece {
            number,
            size,
            hash,
            status: PieceStatus::Missing,
            files: files.to_vec(),
            offset,
            block_count_goal,
            block_count: 0,
        }
    }

    fn add_block(&mut self, offset: usize, block: &[u8]) -> Option<bool> {
        self.block_count += 1;

        let buf = match &self.status {
            PieceStatus::Missing => vec![0u8; self.size.try_into().unwrap()],
            PieceStatus::Awaiting(b) => b.to_vec(),
            PieceStatus::Available => panic!("This piece is already available"),
        };

        let new_buf = [&buf[..offset], &block, &buf[offset + block.len()..]].concat();

        self.status = PieceStatus::Awaiting(new_buf);

        if self.block_count == self.block_count_goal {
            return Some(self.write());
        }
        None
    }

    fn write(&mut self) -> bool {
        //if whole piece is downloaded
        if let PieceStatus::Awaiting(mut buffer) = self.status.clone() {
            let mut offset = self.offset;

            if !self.check_hash(&buffer) {
                self.status = PieceStatus::Missing;
                return false;
            }

            let mut prev_written_bytes = 0;
            for file in &self.files {
                let mut f = OpenOptions::new()
                    .write(true)
                    .open(file.0.as_path())
                    .unwrap();

                f.seek(SeekFrom::Start(offset as u64)).expect("seek failed");

                let how_much =
                    std::cmp::min(file.1 - offset, self.size as usize - prev_written_bytes);
                let written = f.write(&buffer[..how_much]);
                match written {
                    Ok(count) => {
                        prev_written_bytes = count;
                        buffer.drain(..count);
                        offset = 0;
                    }
                    Err(e) => {
                        println!("Write failed\n{:?}", e);
                    }
                }
            }
            self.status = PieceStatus::Available;
            true
        } else {
            panic!("Trying to write a piece with no buffer");
        }
    }

    fn check_hash(&mut self, buffer: &[u8]) -> bool {
        let mut hasher = Sha1::new();
        hasher.update(buffer);
        let hexes = hasher.finalize();
        let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");
        self.status = if hexes == self.hash {
            PieceStatus::Available
        } else {
            PieceStatus::Missing
        };
        hexes == self.hash
    }
}

#[derive(Debug, Clone)]
enum PieceStatus {
    Missing,
    Available,
    Awaiting(Vec<u8>),
}
