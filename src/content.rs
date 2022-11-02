use crate::tf;
use crate::BLOCK_SIZE;
use dirs;
use fs::OpenOptions;
use sha1::Digest;
use sha1::Sha1;
use std::fs;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use tf::TorrentFile;
// use threadpool::ThreadPool;

#[derive(Debug)]
pub struct Content {
    pub pieces: Vec<Mutex<Piece>>,
    files: Vec<(PathBuf, usize)>,
    pub destination_path: String,
    pub events: ContentEvents,
}

impl Content {
    pub fn new(tf: &TorrentFile, dir_path_string: Option<String>) -> Content {
        let dir_path_string = match dir_path_string {
            Some(path) => path,
            None => dirs::download_dir()
                .unwrap()
                .into_os_string()
                .into_string()
                .unwrap(),
        };
        let file_path = format!(
            "{}/{}",
            dir_path_string,
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
            pieces.push(Mutex::new(Piece::new(
                piece_number,
                tf.info.piece_length,
                tf.info.piece_length / BLOCK_SIZE,
                offset,
                piece_files,
                hash,
            )));
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
        //because it's size is unconvential, smaller blocks are needed
        let last_piece_size = tf.info.get_last_piece_size();
        let mut remainder = last_piece_size % BLOCK_SIZE;
        let mut smaller_blocks = 0;
        while remainder != 0 {
            remainder = remainder & (remainder - 1);
            smaller_blocks += 1;
        }
        let block_count_goal = last_piece_size / BLOCK_SIZE + smaller_blocks;

        pieces.push(Mutex::new(Piece::new(
            tf.info.piece_count - 1,
            last_piece_size,
            block_count_goal,
            offset,
            piece_files,
            hash,
        )));

        Content {
            pieces,
            files,
            destination_path: dir_path_string,
            events: ContentEvents::new(),
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
        self.events
            .preallocaion_end
            .iter()
            .for_each(|e| e(self.files.len() as u32));
    }

    pub fn check_content_hash(&self) {
        println!("Checking files hash");

        //todo maybe return this threadPool?
        // let pool = ThreadPool::new(9);

        thread::scope(|s| {
            let mut handles = vec![];
            for piece in &self.pieces {
                let h = s.spawn(move || {
                    let mut read_buf = Vec::with_capacity(piece.lock().unwrap().size as usize);
                    let mut piece = piece.lock().unwrap();
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
                    piece.check_hash(&read_buf);
                });
                handles.push(h);
            }
            for handle in handles {
                let _r = handle.join();
            }
        });
        let mut has = 0;
        for piece in &self.pieces {
            match piece.lock().unwrap().status {
                PieceStatus::Missing => {
                    print!("\x1b[91m");
                }
                PieceStatus::Available => {
                    has += 1;
                    print!("\x1b[92m");
                }
                PieceStatus::Awaiting(_) => unreachable!(),
            }
            print!("{} ", piece.lock().unwrap().number);
        }
        println!("\x1b[0m");
        self.events
            .hash_checked
            .iter()
            .for_each(|e| e(has, self.pieces.len() as u32));
    }

    pub fn add_block(&self, piece_number: usize, offset: usize, block: &[u8]) -> Option<bool> {
        self.pieces[piece_number]
            .lock()
            .unwrap()
            .add_block(offset, block)
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
        for (fnum, item) in files.iter().enumerate() {
            l += item.1;
            if piece * piece_length <= l {
                first_file = fnum;
                break;
            }
        }

        let first_offset = files[first_file].1 - (l - (piece * piece_length));
        let files = &files[first_file..=last_file];
        (first_offset, files.to_vec())
    }

    pub fn get_bitfield(&self) -> Vec<u8> {
        let size = self.pieces.len() / 8 + if self.pieces.len() % 8 == 0 { 0 } else { 1 };
        let mut field = vec![0u8; size];

        for (i, piece) in self.pieces.iter().enumerate() {
            if piece.lock().unwrap().status != PieceStatus::Available {
                continue;
            }
            field[i / 8] += (128 / 2u32.pow(i as u32 % 8)) as u8;
        }

        field
    }
}

// #[derive(Debug)]
pub struct ContentEvents {
    pub preallocaion_start: Vec<Box<dyn Fn(u32) + 'static + Send + Sync>>,
    pub preallocaion_end: Vec<Box<dyn Fn(u32) + 'static + Send + Sync>>,
    pub hash_checked: Vec<Box<dyn Fn(u32, u32) + 'static + Send + Sync>>,
}

impl ContentEvents {
    pub fn new() -> ContentEvents {
        ContentEvents {
            preallocaion_start: vec![],
            preallocaion_end: vec![],
            hash_checked: vec![],
        }
    }
}

impl Default for ContentEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ContentEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}{:?}",
            self.preallocaion_start.len(),
            self.preallocaion_end.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub number: u32,
    size: u32,
    offset: usize,
    pub status: PieceStatus,
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
        block_count_goal: u32,
        offset: usize,
        files: Vec<(PathBuf, usize)>,
        hash: [u8; 20],
    ) -> Piece {
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

        let new_buf = [&buf[..offset], block, &buf[offset + block.len()..]].concat();

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
        if hexes == self.hash {
            self.status = PieceStatus::Available;
        } else {
            self.status = PieceStatus::Missing;
            self.block_count = 0;
        };
        hexes == self.hash
    }

    pub fn make_awaiting(&mut self) {
        if self.status != PieceStatus::Missing {
            return;
        };
        self.status = PieceStatus::Awaiting(vec![0u8; self.size.try_into().unwrap()]);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PieceStatus {
    Missing,
    Available,
    Awaiting(Vec<u8>),
}
