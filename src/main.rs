use bendy::decoding::FromBencode;
use sha1::{Digest, Sha1};
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::{stdout, ErrorKind, Read, Seek, SeekFrom, Write};
use std::net::TcpStream;
use std::slice::Iter;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;
use threadpool::ThreadPool;

mod tf;
use crate::tf::*;
mod tracker;
use crate::tracker::*;

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

    let streams = connect_to_peers(respone, &tf);
    let mut streams = streams.iter();

    while missing_pieces.len() > 0 {
        let s = streams.next();
        download_from_peer(&s.unwrap().stream, &tf, missing_pieces.iter())
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

fn connect_to_peers(respone: TrackerResponse, tf: &TorrentFile) -> Vec<Peer> {
    let mut streams = vec![];
    let pool = ThreadPool::new(9);
    let (tx, rx) = channel();
    let respone = Arc::new(respone);

    #[derive(Debug)]
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

        pool.execute(move || {
            stdout().flush().unwrap();
            let stream = TcpStream::connect_timeout(&respone.peers[i], Duration::from_secs(2));

            if let Ok(mut s) = stream {
                s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
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
                tx.send((Result::Done(Peer {
                    id: peer_id,
                    stream: s,
                    bitfield: vec![],
                    status: (true, false, true, false)
                }), respone.peers[i]))
                    .expect("channel will be there waiting for the pool");
            } else {
                tx.send((Result::Timeout, respone.peers[i]))
                    .expect("channel will be there waiting for the pool");
            }
        });
    }

    rx.iter().take(respone.peers.len()).for_each(|(res, peer)| {
        print!("{} ", peer);
        match res {
            Result::Done(s) => {
                println!("\x1b[1mDone!\x1b[0m");
                streams.push(s);
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

fn download_from_peer(mut s: &TcpStream, tf: &TorrentFile, mut missing_pieces: Iter<usize>) {
    //reading actuall data
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap(); //safe unwrap

    //am_choking = 1, am_interested = 0, peer_choking = 1, peer_interested = 0
    let mut peer_status = (true, false, true, false);
    let mut pn = missing_pieces.next().unwrap();
    let mut pl: u32 = tf.info.piece_length as u32;
    let piece_count = tf.info.length / tf.info.piece_length as usize;
    //16Kb more than that doesn't work somehow, BEP 52 or something,
    let mut block_size: u32 = 16384;
    let mut piece: Vec<u8> = vec![];
    let mut offset: u32 = 0;

    loop {
        let mut message_size = [0u8; 4];
        let package_size = s.read(&mut message_size);
        match package_size {
            Ok(_package_size) => {} //println!("package_size {}", package_size),
            Err(e) => {
                println!("No package for 5 secs; {}", e);
                if peer_status.2 == true {
                    println!("Sending unchoke and interested");
                    //send unchoke and interested
                    let r = s.write(&[0, 0, 0, 1, 1, 0, 0, 0, 1, 2]);
                    match r {
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => println!("Error writing buffer: {:?}", e),
                        _ => {}
                    }
                    peer_status.0 = false;
                    peer_status.1 = true;
                } else {
                    println!("Send keep-alive");
                    let r = s.write(&[0, 0, 0, 0]);
                    match r {
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) if e.kind() == ErrorKind::ConnectionAborted => {
                            println!("\x1b[91mConnection aborted\x1b[0m");
                        }
                        Err(e) => println!("Error writing buffer: {:?}", e),
                        _ => {}
                    }
                }
            }
        }

        let message_size = big_endian_to_u32(&message_size);
        if message_size == 0 {
            println!("keep alive");
            continue;
        }

        let mut message_buf = vec![0u8; message_size as usize];
        s.read_exact(&mut message_buf)
            .expect("Couldn't read buffer");
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
                println!("choke");
                peer_status.2 = true;
                println!("Sending unchoke and interested");
                //send unchoke and interested
                let r = s.write(&[0, 0, 0, 1, 1, 0, 0, 0, 1, 2]);
                match r {
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => println!("Error writing buffer: {:?}", e),
                    _ => {}
                }
                peer_status.0 = false;
                peer_status.1 = true;
            }
            1 => {
                println!("unchoke");
                peer_status.2 = false;
                println!("\x1b[1mRequesting piece {}\x1b[0m", pn);
            }
            2 => println!("interested"),
            3 => println!("not interested"),
            4 => println!("have\n {:?}", &message_buf[1..]),
            5 => println!("bitfield\n {:?}", &message_buf[1..]),
            6 => println!("request"),
            7 => {
                print!(
                    "\x1b[1M\rGot piece {} of {} ({}%), offset {}/{} ({}%)",
                    big_endian_to_u32(&message_buf[1..5].try_into().unwrap()),
                    piece_count,
                    (big_endian_to_u32(&message_buf[1..5].try_into().unwrap()) as f32
                        / ((tf.info.length as f32 / tf.info.piece_length as f32) / 100.0))
                        as u32,
                    big_endian_to_u32(&message_buf[5..9].try_into().unwrap()),
                    pl,
                    ((big_endian_to_u32(&message_buf[5..9].try_into().unwrap()) + block_size)
                        as f32
                        / (pl as f32 / 100.0)) as u32
                );
                stdout().flush().unwrap();
                piece.append(&mut message_buf[9..].to_vec());

                //if whole piece is downloaded
                if piece.len() == pl.try_into().unwrap() {
                    //check hash
                    let mut hasher = Sha1::new();
                    hasher.update(&piece);
                    let hexes = hasher.finalize();
                    let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");
                    if hexes != tf.info.get_piece_hash(*pn) {
                        println!(" \x1b[91mHash doesn't match!\x1b[0m");
                        offset = 0;
                        continue;
                    } else {
                        println!(" \x1b[92mCorrect hash!\x1b[0m");
                    }
                    //==========
                    let (mut of, files) = tf.info.get_piece_files(*pn);

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
                        let written = f.write(&piece[..how_much]);
                        match written {
                            Ok(count) => {
                                prev_written_bytes = count;
                                piece.drain(..count);
                                of = 0;
                            }
                            Err(e) => {
                                println!("Write failed\n{:?}", e);
                            }
                        }
                        //println!("Piece {:?}", &piece);
                    }
                    //=======
                    /*
                    println!(
                        "SEEEEEK OFFSET {:?}",
                        *pn as u64 * (tf.info.piece_length as u64)
                    );
                    file.seek(SeekFrom::Start(*pn as u64 * (tf.info.piece_length as u64)))
                        .expect("seek failed");
                    file.write_all(&piece).expect("write failed");
                    */
                    piece.drain(..);

                    pn = missing_pieces.next().unwrap();
                    offset = 0;
                    println!("\x1b[1mRequesting piece {}\x1b[0m", pn);

                    if pn > &piece_count {
                        break;
                    }
                }
                //println!("");
            }
            8 => println!("cancel"),
            9 => println!("port"),
            _ => println!("WHAT?!"),
        }

        //request pieces if we are unchoked
        if peer_status.2 == false && offset < pl && pn <= &piece_count {
            let mut request_message = vec![0, 0, 0, 13, 6]; //constant part
            request_message.append(&mut (*pn as u32).to_be_bytes().to_vec());
            let be_offset = offset.to_be_bytes();
            request_message.append(&mut be_offset.to_vec()); //piece uhh, offset?

            if *pn == piece_count {
                if offset == 0 {
                    pl = (tf.info.length - ((piece_count) * pl as usize)) as u32;
                }
                let left = pl - offset;
                block_size = if left < 16384 {
                    //bitwise magic! this finds the rightmost bit it last_piece_size
                    // pl & (!(pl - 1))
                    //and this finds the rightmost bit it last_piece_size
                    1 << (31 - left.leading_zeros())
                } else {
                    16384
                };
            }
            //piece length
            request_message.append(&mut block_size.to_be_bytes().to_vec());
            /*println!(
                "\x1b[1mRequesting piece {}\x1b[0m, offset {}, block size {} bytes",
                pn, offset, block_size
            );*/
            match s.write(&request_message) {
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    println!("\x1b[91mError writing buffer: {}\x1b[0m", e.to_string());
                    continue;
                }
                _ => {}
            }
            offset += block_size;
        }
    }
}

//todo check this fn it can be better, this is way too slow (268435456)
fn check_file_hash(tf: &TorrentFile) -> Vec<usize> {
    println!("Checking files hash");
    let piece_count = tf.info.length / tf.info.piece_length as usize;
    let mut missing_pieces = vec![];

    let pool = ThreadPool::new(9);

    let (tx, rx) = channel();

    for p in 0..piece_count + 1 {
        let mut read_buf = Vec::with_capacity(tf.info.piece_length as usize);
        let (offset, files) = tf.info.get_piece_files(p);

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

    rx.iter().take(piece_count + 1).for_each(|(p, hexes)| {
        if hexes != tf.info.get_piece_hash(p) {
            print!("\x1b[91m");
            missing_pieces.push(p);
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

fn try_parse_client(peer_info: &[u8]) -> String {
    let huh = [peer_info[1], peer_info[2]];

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
        _ => String::from("unknown client"),
    }
}

#[derive(Debug)]
struct Peer {
    id: [u8;20],
    stream: TcpStream,
    bitfield: Vec<u8>,
    status: (bool,bool,bool,bool)
}

impl Peer {
    // add code here
}