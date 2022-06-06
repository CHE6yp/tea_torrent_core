use bendy::decoding::FromBencode;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::{stdout, ErrorKind, Read, Result, Seek, SeekFrom, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use threadpool::ThreadPool;

mod tf;
use crate::tf::*;
mod tracker;
use crate::tracker::*;

const BLOCK_SIZE: u32 = 16384;

fn main() {
    println!("\x1b]0;tTorrent\x07");
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x).unwrap();
    println!("{}", tf);

    preallocate(&tf);
    let missing_pieces = check_file_hash(&tf);

    let r = connect_to_tracker(&tf, false);
    if r.is_none() {
        println!("Connection failed");
        return;
    }

    let respone = r.unwrap();
    println!("Connection complete, connecting to peers");

    let mut peers = connect_to_peers(respone, &tf);
    let mut handles: Vec<thread::JoinHandle<_>> = vec![];

    let tf = Arc::new(tf);
    let (tx, rx) = channel();

    for peer in &peers {
        let peer = Arc::clone(&peer);
        let tx = tx.clone();
        let join_handle = thread::Builder::new()
            .name(peer.id_string())
            .spawn(move || loop {
                let r = peer.get_message();
                if !r.is_none() {
                    //TODO wanna send this inside get_message()
                    let _r = tx.send((Arc::clone(&peer), r.unwrap()));
                }
            })
            .unwrap();

        handles.push(join_handle);
    }

    let tf_piece = Arc::clone(&tf);
    handles.push(thread::spawn(move || {
        let mut pieces = HashMap::new();
        rx.iter().for_each(|(peer, piece_buf)| {
            let pn = big_endian_to_u32(&piece_buf[1..5].try_into().unwrap());
            if !pieces.contains_key(&pn) {
                let piece_length = if pn == &tf_piece.info.piece_count - 1 {
                    tf_piece.info.get_last_piece_size()
                } else {
                    tf_piece.info.piece_length
                };
                pieces.insert(pn, Piece::new(piece_length));
            }
            pieces.get_mut(&pn).unwrap().add_block(piece_buf);
            if pieces[&pn].block_count == pieces[&pn].block_count_goal {
                pieces[&pn].write(pn, peer, &tf_piece);
                pieces.remove(&pn);
            }
        });
        println!("Write thread DONE!");
    }));

    let mut missing_pieces = missing_pieces.iter();
    let mut piece = missing_pieces.next();

    while piece != None {
        let p = piece.unwrap();

        let peersclone = peers.clone();
        let peersclone = peersclone
            .into_iter()
            .filter(|peer| peer.has_piece(*p) && *peer.busy.lock().unwrap() == false)
            .collect::<Vec<Arc<Peer>>>();
        let peer = peersclone.first();

        if peer.is_none() {
            continue;
        }

        let peer = Arc::clone(&peer.unwrap());

        let piece_length = if *p == tf.info.piece_count as usize - 1 {
            //last piece
            tf.info.length as u32 - (tf.info.piece_length as u32 * (*p as u32))
        } else {
            tf.info.piece_length
        };
        let res = peer.request(&peer.stream, *p as u32, piece_length);
        match res {
            Ok(true) => piece = missing_pieces.next(),
            Ok(false) => (),
            Err(_e) => {
                println!("\x1b[91mRemoving peer {} \x1b[0m", peer.id_string());
                let index = peersclone.iter().position(|x| x.id == peer.id).unwrap();
                peers.remove(index);
            }
        }
    }
    println!("missing_pieces DONE!");

    for handle in handles {
        let _r = handle.join();
    }
}

fn connect_to_peers(respone: TrackerResponse, tf: &TorrentFile) -> Vec<Arc<Peer>> {
    let mut streams = vec![];
    let pool = ThreadPool::new(9);
    let (tx, rx) = channel();
    let respone = Arc::new(respone);

    enum Result {
        Done(Peer),
        Timeout,
        InvalidHash,
    }

    //TODO look into arcing this instead if cloning
    //maybe i need scoped threads?
    let mut info_hash = [0; 20];
    info_hash.clone_from_slice(tf.info_hash.raw());

    for i in 0..respone.peers.len() {
        let respone = Arc::clone(&respone);
        let tx = tx.clone();
        let piece_count = tf.info.piece_count;

        pool.execute(move || {
            stdout().flush().unwrap();
            let stream = TcpStream::connect_timeout(&respone.peers[i], Duration::from_secs(10));

            if let Ok(mut s) = stream {
                //s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
                //writing handhsake
                let mut arr = vec![19];
                arr.extend(b"BitTorrent protocol");
                arr.extend([0, 0, 0, 0, 0, 0, 0, 0]);
                arr.extend(info_hash);
                arr.extend(b"-tT0001-004815162342"); //12 rand numbers at the end TODO
                let _r = s.write(&arr).expect("Couldn't write buffer; Handshake");
                //reading handshake
                let mut handshake_buff = [0u8; 68];
                let _r = s.read(&mut handshake_buff);

                if let Err(_e) = _r {
                    tx.send((Result::Timeout, respone.peers[i]))
                        .expect("channel will be there waiting for the pool");
                    return;
                }
                if &handshake_buff[28..48] != info_hash {
                    tx.send((Result::InvalidHash, respone.peers[i]))
                        .expect("channel will be there waiting for the pool");
                    return;
                }
                let mut peer_id = [0; 20];
                peer_id.clone_from_slice(&handshake_buff[48..68]);
                // s.set_nonblocking(true);
                tx.send((
                    Result::Done(Peer {
                        id: peer_id,
                        stream: s,
                        bitfield: Mutex::new(vec![0; piece_count as usize]),
                        status: Mutex::new((true, false, true, false)),
                        busy: Mutex::new(false),
                    }),
                    respone.peers[i],
                ))
                .expect("channel will be there waiting for the pool");
            } else {
                tx.send((Result::Timeout, respone.peers[i]))
                    .expect("channel will be there waiting for the pool");
            }
        });
    }

    rx.iter().take(respone.peers.len()).for_each(|(res, peer)| {
        print!("{} ", peer);
        //TODO maybe need a result instead of just enum?
        match res {
            Result::Done(s) => {
                println!("\x1b[1mDone!\x1b[0m {}", s.try_parse_client());
                streams.push(Arc::new(s));
            }
            Result::Timeout => {
                println!("\x1b[91mFailed!\x1b[0m");
            }
            Result::InvalidHash => {
                // println!(
                //     "{};\n extentions {:?}\n info_hash {}\n vs our    {}\n peer id {} ({})",
                //     String::from_utf8_lossy(&handshake_buff[1..20]),
                //     &handshake_buff[20..28],
                //     String::from_utf8_lossy(&handshake_buff[28..48]),
                //     String::from_utf8_lossy(&info_hash),
                //     String::from_utf8_lossy(&handshake_buff[48..68]),
                //     try_parse_client(&handshake_buff[48..68])
                // );
                println!("\x1b[91mInvalid info hash!\x1b[0m");
            }
        }
    });
    streams
}

fn check_file_hash(tf: &TorrentFile) -> Vec<usize> {
    println!("Checking files hash");
    let mut missing_pieces = vec![];

    let pool = ThreadPool::new(9);

    let (tx, rx) = channel();

    for p in 0..tf.info.piece_count {
        let mut read_buf = Vec::with_capacity(tf.info.piece_length as usize);
        let (offset, files) = tf.info.get_piece_files(p as usize);

        let mut first = true;
        let mut r = 0;
        for file in files {
            let file_path = format!(
                "downloads/{}",
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
                print!("\x1b[92m");
            }
            print!("{} ", p);
            stdout().flush().unwrap();
        });

    println!("\x1b[0m");
    missing_pieces
}

fn big_endian_to_u32(value: &[u8; 4]) -> u32 {
    ((value[0] as u32) << 24)
        + ((value[1] as u32) << 16)
        + ((value[2] as u32) << 8)
        + value[3] as u32
}

#[derive(Debug)]
struct Peer {
    id: [u8; 20],
    stream: TcpStream,
    bitfield: Mutex<Vec<u8>>,
    //am_choking = 1, am_interested = 0, peer_choking = 1, peer_interested = 0
    status: Mutex<(bool, bool, bool, bool)>,
    busy: Mutex<bool>,
}

impl Peer {
    fn get_message(&self) -> Option<Vec<u8>> {
        //self.stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        let mut message_size = [0u8; 4];
        let mut stream = &self.stream;
        let package_size = stream.read(&mut message_size);
        match package_size {
            Ok(_package_size) => {} //println!("package_size {}", package_size),
            Err(e) => {
                println!("No package for some secs; read timeout; {}", e);

                println!("Send keep-alive");
                let r = stream.write(&[0, 0, 0, 0]);
                match r {
                    Err(e) if e.kind() == ErrorKind::Interrupted => {
                        println!("\x1b[91mInterrupted\x1b[0m {}", self.id_string());
                        return None;
                    }
                    Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                        println!("\x1b[91mConnection Reset\x1b[0m {}", self.id_string());
                        panic!("{:?}", e.kind());
                    }
                    Err(e) if e.kind() == ErrorKind::ConnectionAborted => {
                        println!("\x1b[91mConnection aborted\x1b[0m {}", self.id_string());
                        // return Message::Aborted;
                        panic!("{:?}", e.kind());
                    }
                    Err(e) => println!("Error writing buffer: {:?}", e),
                    _ => {}
                }
            }
        }

        let message_size = big_endian_to_u32(&message_size);
        if message_size == 0 {
            // println!("Got keep alive");
            return None;
        }

        let mut message_buf = vec![0u8; message_size as usize];
        let r = stream.read_exact(&mut message_buf);
        if let Err(e) = r {
            panic!(
                "Couldn't read buffer; {:?}\n message_size {} ",
                e, message_size,
            );
        }
        /*
            keep-alive: <len=0000>
            choke: <len=0001><id=0>
            unchoke: <len=0001><id=1>
            interested: <len=0001><id=2>
            not interested: <len=0001><id=3>
            have: <len=0005><id=4><piece index>
            bitfield: <len=0001+X><id=5><bitfield>
            request: <len=0013><id=6><index><begin><length>
            piece: <len=0009+X><id=7><index><begin><block>
            cancel: <len=0013><id=8><index><begin><length>
            port: <len=0003><id=9><listen-port>
        */
        match &message_buf[0] {
            0 => {
                println!("Choked by {}", self.id_string());
                self.status.lock().unwrap().2 = true;
            }
            1 => {
                println!("Unchoked by {}", self.id_string());
                self.status.lock().unwrap().2 = false;
                *self.busy.lock().unwrap() = false;
            }
            2 => {
                println!("interested");
            }
            3 => {
                println!("not interested");
            }
            4 => {
                // println!("have {:?} {}", &message_buf[1..], self.id_string());
                self.add_piece_to_bitfield(big_endian_to_u32(
                    &message_buf[1..].try_into().unwrap(),
                ));
            }
            5 => {
                *self.bitfield.lock().unwrap() = (&message_buf[1..]).to_vec();
                // println!("{} bitfield\n {:?}", self.id_string(), &message_buf[1..]);
            }
            6 => {
                println!("request");
            }
            7 => {
                //println!("Piece");
                return Some(message_buf);
                //tx.send((&self, message_buf));
            }
            8 => {
                println!("cancel");
            }
            9 => {
                println!("port");
            }
            _ => {
                panic!("Unknown message!");
            }
        }
        None
    }

    fn request(
        &self,
        mut stream: &TcpStream,
        piece_number: u32,
        piece_length: u32,
    ) -> Result<bool> {
        // if self.status.lock().unwrap().1 && self.status.lock().unwrap().2 {
        // println!("peer {} stat {:?}, val {:?}", self.id_string(), self.status, piece_number);
        if let Ok(mut st) = self.status.lock() {
            if !st.1 && st.2 {
                // if !(self.status.lock().unwrap().1) && self.status.lock().unwrap().2 {
                println!("Sending unchoke and interested");
                //send unchoke and interested
                let r = stream.write(&[0, 0, 0, 1, 1, 0, 0, 0, 1, 2]);
                match r {
                    Err(e) if e.kind() == ErrorKind::Interrupted => {
                        println!("\x1b[91mInterrupted\x1b[0m {}", self.id_string());
                        return Ok(false);
                    }
                    Err(e) => println!("Error writing buffer: {:?}", e),
                    _ => {}
                }
                st.0 = false;
                st.1 = true;
                *self.busy.lock().unwrap() = true;
                return Ok(false);
            }
        }

        if self.status.lock().unwrap().2 {
            // println!("Sstill choked");
            return Ok(false);
        }

        println!("Request {} from {}", piece_number, self.id_string());
        let mut offset: u32 = 0;
        let mut left = piece_length;
        *self.busy.lock().unwrap() = true;

        while left > 0 {
            let block_size = if left < BLOCK_SIZE {
                //bitwise magic! this finds the rightmost bit it last_piece_size
                // piece_length & (!(piece_length - 1))
                //and this finds the rightmost bit it last_piece_size
                1 << (31 - left.leading_zeros())
            } else {
                BLOCK_SIZE
            };

            let mut request_message = vec![0, 0, 0, 13, 6]; //constant part
            request_message.append(&mut piece_number.to_be_bytes().to_vec());
            let be_offset = offset.to_be_bytes();
            request_message.append(&mut be_offset.to_vec()); //piece uhh, offset?
            request_message.append(&mut block_size.to_be_bytes().to_vec());
            // println!("request_message {:?}", request_message);
            // println!("Request message {:?} from {}", &request_message, self.id_string());
            match stream.write(&request_message) {
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    println!("\x1b[91mError writing buffer: {:?}\x1b[0m", e);
                }
                _ => {}
            }

            left -= block_size;
            offset += block_size;
        }
        Ok(true)
    }

    fn try_parse_client(&self) -> String {
        let huh = [self.id[1], self.id[2]];

        match &huh {
            b"7T" => String::from("aTorrent for Android"),
            b"AB" => String::from("AnyEvent::BitTorrent"),
            b"AG" => String::from("Ares"),
            b"A~" => String::from("Ares"),
            b"AR" => String::from("Arctic"),
            b"AV" => String::from("Avicora"),
            b"AT" => String::from("Artemis"),
            b"AX" => String::from("BitPump"),
            b"AZ" => String::from("Azureus"),
            b"BB" => String::from("BitBuddy"),
            b"BC" => String::from("BitComet"),
            b"BE" => String::from("Baretorrent"),
            b"BF" => String::from("Bitflu"),
            b"BG" => String::from("BTG (uses Rasterbar libtorrent)"),
            b"BL" => String::from("BitCometLite (uses 6 digit version number) or BitBlinder"),
            b"BP" => String::from("BitTorrent Pro (Azureus + spyware)"),
            b"BR" => String::from("BitRocket"),
            b"BS" => String::from("BTSlave"),
            b"BT" => String::from("mainline BitTorrent (versions >= 7.9) or BBtor"),
            b"Bt" => String::from("Bt"),
            b"BW" => String::from("BitWombat"),
            b"BX" => String::from("~Bittorrent X"),
            b"CD" => String::from("Enhanced CTorrent"),
            b"CT" => String::from("CTorrent"),
            b"DE" => String::from("DelugeTorrent"),
            b"DP" => String::from("Propagate Data Client"),
            b"EB" => String::from("EBit"),
            b"ES" => String::from("electric sheep"),
            b"FC" => String::from("FileCroc"),
            b"FD" => String::from("Free Download Manager (versions >= 5.1.12)"),
            b"FT" => String::from("FoxTorrent"),
            b"FX" => String::from("Freebox BitTorrent"),
            b"GS" => String::from("GSTorrent"),
            b"HK" => String::from("Hekate"),
            b"HL" => String::from("Halite"),
            b"HM" => String::from("hMule (uses Rasterbar libtorrent)"),
            b"HN" => String::from("Hydranode"),
            b"IL" => String::from("iLivid"),
            b"JS" => String::from("Justseed.it client"),
            b"JT" => String::from("JavaTorrent"),
            b"KG" => String::from("KGet"),
            b"KT" => String::from("KTorrent"),
            b"LC" => String::from("LeechCraft"),
            b"LH" => String::from("LH-ABC"),
            b"LP" => String::from("Lphant"),
            b"LT" => String::from("libtorrent"),
            b"lt" => String::from("libTorrent"),
            b"LW" => String::from("LimeWire"),
            b"MK" => String::from("Meerkat"),
            b"MO" => String::from("MonoTorrent"),
            b"MP" => String::from("MooPolice"),
            b"MR" => String::from("Miro"),
            b"MT" => String::from("MoonlightTorrent"),
            b"NB" => String::from("Net::BitTorrent"),
            b"NX" => String::from("Net Transport"),
            b"OS" => String::from("OneSwarm"),
            b"OT" => String::from("OmegaTorrent"),
            b"PB" => String::from("Protocol::BitTorrent"),
            b"PD" => String::from("Pando"),
            b"PI" => String::from("PicoTorrent"),
            b"PT" => String::from("PHPTracker"),
            b"qB" => String::from("qBittorrent"),
            b"QD" => String::from("QQDownload"),
            b"QT" => String::from("Qt 4 Torrent example"),
            b"RT" => String::from("Retriever"),
            b"RZ" => String::from("RezTorrent"),
            b"S~" => String::from("Shareaza alpha/beta"),
            b"SB" => String::from("~Swiftbit"),
            b"SD" => String::from("Thunder (aka XùnLéi)"),
            b"SM" => String::from("SoMud"),
            b"SP" => String::from("BitSpirit"),
            b"SS" => String::from("SwarmScope"),
            b"ST" => String::from("SymTorrent"),
            b"st" => String::from("sharktorrent"),
            b"SZ" => String::from("Shareaza"),
            b"TB" => String::from("Torch"),
            b"TE" => String::from("terasaur Seed Bank"),
            b"TL" => String::from("Tribler (versions >= 6.1.0)"),
            b"TN" => String::from("TorrentDotNET"),
            b"TR" => String::from("Transmission"),
            b"TS" => String::from("Torrentstorm"),
            b"TT" => String::from("TuoTu"),
            b"UL" => String::from("uLeecher!"),
            b"UM" => String::from("µTorrent for Mac"),
            b"UT" => String::from("µTorrent"),
            b"VG" => String::from("Vagaa"),
            b"WD" => String::from("WebTorrent Desktop"),
            b"WT" => String::from("BitLet"),
            b"WW" => String::from("WebTorrent"),
            b"WY" => String::from("FireTorrent"),
            b"XF" => String::from("Xfplay"),
            b"XL" => String::from("Xunlei"),
            b"XS" => String::from("XSwifter"),
            b"XT" => String::from("XanTorrent"),
            b"XX" => String::from("Xtorrent"),
            b"ZT" => String::from("ZipTorrent"),
            b"ZO" => String::from("ZONA ?"),
            _ => String::from("unknown client"),
        }
    }

    fn id_string(&self) -> String {
        format!(
            "{}:{}.{}.{}.{}.{}.{}.{}.{}.{}.{}.{}.{}",
            String::from_utf8_lossy(&self.id[..8]),
            &self.id[8],
            &self.id[9],
            &self.id[10],
            &self.id[11],
            &self.id[12],
            &self.id[13],
            &self.id[14],
            &self.id[15],
            &self.id[16],
            &self.id[17],
            &self.id[18],
            &self.id[19],
        )
    }

    fn has_piece(&self, piece_number: usize) -> bool {
        let byte = piece_number / 8;
        let bit = (piece_number % 8) as u8;
        let field = (*self.bitfield.lock().unwrap())[byte as usize];
        match bit {
            0 => field & 0b10000000 == 0b10000000,
            1 => field & 0b01000000 == 0b01000000,
            2 => field & 0b00100000 == 0b00100000,
            3 => field & 0b00010000 == 0b00010000,
            4 => field & 0b00001000 == 0b00001000,
            5 => field & 0b00000100 == 0b00000100,
            6 => field & 0b00000010 == 0b00000010,
            7 => field & 0b00000001 == 0b00000001,
            _ => unreachable!(),
        }
    }

    fn add_piece_to_bitfield(&self, piece_number: u32) {
        let byte = piece_number / 8;
        let bit = (piece_number % 8) as u8;
        self.bitfield.lock().unwrap()[byte as usize] |= match bit {
            0 => 0b10000000,
            1 => 0b01000000,
            2 => 0b00100000,
            3 => 0b00010000,
            4 => 0b00001000,
            5 => 0b00000100,
            6 => 0b00000010,
            7 => 0b00000001,
            _ => unreachable!(),
        };
    }
}

fn preallocate(tf: &TorrentFile) {
    println!("Preallocating files");
    let file_path = format!(
        "downloads/{}",
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

#[derive(Debug)]
struct Piece {
    buf: Vec<u8>,
    block_count: u32,
    block_count_goal: u32,
}

impl Piece {
    fn new(size: u32) -> Piece {
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
            buf: vec![0; size.try_into().unwrap()],
            block_count_goal: block_count_goal,
            block_count: 0,
        }
    }

    fn add_block(&mut self, block: Vec<u8>) {
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
    }

    fn write(&self, piece_number: u32, peer: Arc<Peer>, tf: &TorrentFile) -> bool {
        //if whole piece is downloaded
        let mut buffer = self.buf.clone();
        //check hash
        let mut hasher = Sha1::new();
        hasher.update(&buffer);
        let hexes = hasher.finalize();
        let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");
        if hexes != tf.info.get_piece_hash(piece_number as usize) {
            println!(
                " \x1b[91mHash doesn't match!\x1b[0m Piece {} from{}",
                piece_number,
                peer.id_string(),
            );
            *peer.busy.lock().unwrap() = false;
            return false;
        }

        println!(
            " \x1b[92mCorrect hash!\x1b[0m Piece {} from {}",
            piece_number,
            peer.id_string(),
        );
        //==========
        let (mut of, files) = tf.info.get_piece_files(piece_number as usize);

        let mut prev_written_bytes = 0;
        for file in files {
            let file_path = format!(
                "downloads/{}",
                if tf.info.files.len() > 1 {
                    &tf.info.name
                } else {
                    ""
                }
            );
            let path = std::path::Path::new(&file_path);
            let mut fc = file.clone();
            fc.path = path.join(file.path.clone());
            let mut f = OpenOptions::new().write(true).open(fc.path).unwrap();

            f.seek(SeekFrom::Start(of as u64)).expect("seek failed");

            let how_much = std::cmp::min(
                file.length - of,
                tf.info.piece_length as usize - prev_written_bytes,
            );
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

        *peer.busy.lock().unwrap() = false;
        true
    }
}
