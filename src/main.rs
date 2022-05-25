use bendy::decoding::FromBencode;
use sha1::{Digest, Sha1};
use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::net::TcpStream;
use std::slice::Iter;
use std::time::Duration;

mod tf;
use crate::tf::*;
mod tracker;
use crate::tracker::*;

fn main() {
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x).unwrap();
    println!("\n{}\n", tf);

    let mut file;
    let file_path = format!("downloads/{}", &tf.info.name);
    let path = std::path::Path::new(&file_path);
    return;

    if path.exists() {
        let mut oo = OpenOptions::new();
        oo.read(true);
        oo.write(true);

        let file_length = path.metadata().unwrap().len();
        if file_length < tf.info.length.try_into().unwrap() {
            oo.append(true);
        }
        file = oo.open(file_path).unwrap();

        if file_length < tf.info.length.try_into().unwrap() {
            file.write_all(&vec![0; tf.info.length - file_length as usize])
                .unwrap();
        }
    } else {
        //preallocate file
        file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file_path)
            .unwrap();
        file.write_all(&vec![0; tf.info.length]).unwrap();
    }

    let missing_pieces = check_file_hash(&file, &tf);
    println!("missing_pieces {:?}", missing_pieces);

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
        download_from_peer(s.unwrap(), &file, &tf, missing_pieces.iter())
    }
}

fn connect_to_peers(respone: TrackerResponse, tf: &TorrentFile) -> Vec<TcpStream> {
    let mut streams = vec![];

    for i in 0..respone.peers.len() {
        println!("{}", respone.peers[i]);
    }
    println!("=================");
    for i in 0..respone.peers.len() {
        println!("Trying {}", respone.peers[i]);
        let stream = TcpStream::connect_timeout(&respone.peers[i], Duration::from_secs(2));

        if let Ok(mut s) = stream {
            //writing handhsake
            let mut arr = vec![19];
            arr.extend(b"BitTorrent protocol");
            arr.extend([0, 0, 0, 0, 0, 0, 0, 0]);
            arr.extend(tf.info_hash.raw());
            arr.extend(b"-tT0001-004815162342"); //12 rand numbers at the end TODO
            s.write(&arr).expect("Couldn't write buffer; Handshake"); //28

            //reading handshake
            let mut handshake_buff = [0u8; 68];
            s.read(&mut handshake_buff)
                .expect("Couldn't read buffer; Handshake");
            println!(
                "{};\n extentions {:?}\n info_hash {}\n vs our    {}\n peer id {} ({})",
                String::from_utf8_lossy(&handshake_buff[1..20]),
                &handshake_buff[20..28],
                String::from_utf8_lossy(&handshake_buff[28..48]),
                String::from_utf8_lossy(tf.info_hash.raw()),
                String::from_utf8_lossy(&handshake_buff[48..68]),
                try_parse_client(&handshake_buff[48..68])
            );

            if &handshake_buff[28..48] != tf.info_hash.raw() {
                println!("Invalid info hash!");
                continue;
            }

            streams.push(s);
        } else {
            println!("Failed!");
        }
    }
    streams
}

fn download_from_peer(
    mut s: &TcpStream,
    mut file: &File,
    tf: &TorrentFile,
    mut missing_pieces: Iter<usize>,
) {
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
            }
            2 => println!("interested"),
            3 => println!("not interested"),
            4 => println!("have\n {:?}", &message_buf[1..]),
            5 => println!("bitfield\n {:?}", &message_buf[1..]),
            6 => println!("request"),
            7 => {
                println!(
                    "Got piece {} of {} ({}%), offset {}/{} ({}%)",
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
                piece.append(&mut message_buf[9..].to_vec());

                println!("piece.len() {:?} == piece_length {}", piece.len(), pl);

                //if whole piece is downloaded
                if piece.len() == pl.try_into().unwrap() {
                    //check hash
                    let mut hasher = Sha1::new();
                    hasher.update(&piece);
                    let hexes = hasher.finalize();
                    let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");
                    if hexes != tf.info.get_piece_hash(*pn) {
                        println!("Hash doesn't match!");
                        offset = 0;
                        continue;
                    } else {
                        println!("Correct hash!");
                    }

                    println!(
                        "SEEEEEK OFFSET {:?}",
                        *pn as u64 * (tf.info.piece_length as u64)
                    );
                    file.seek(SeekFrom::Start(*pn as u64 * (tf.info.piece_length as u64)))
                        .expect("seek failed");
                    file.write_all(&piece).expect("write failed");
                    piece.drain(..);

                    pn = missing_pieces.next().unwrap();
                    offset = 0;

                    if pn > &piece_count {
                        break;
                    }
                }
                println!("");
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
            request_message.append(&mut block_size.to_be_bytes().to_vec()); //piece length
            println!(
                "\x1b[1mRequesting piece {}\x1b[0m, offset {}, block size {} bytes",
                pn, offset, block_size
            );
            match s.write(&request_message) {
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    println!("Error writing buffer: {:?}", e);
                    continue;
                }
                _ => {}
            }
            offset += block_size;
        }
    }
}

//todo check this fn it can be better
fn check_file_hash(file: &File, tf: &TorrentFile) -> Vec<usize> {
    let piece_count = tf.info.length / tf.info.piece_length as usize;
    let mut read_buf = Vec::with_capacity(tf.info.piece_length as usize);
    let mut missing_pieces = vec![];
    let mut hasher = Sha1::new();

    for p in 0..piece_count + 1 {
        file.take(tf.info.piece_length as u64)
            .read_to_end(&mut read_buf)
            .unwrap();
        hasher.update(&read_buf);
        let hexes = hasher.finalize_reset();

        read_buf.drain(..);
        let hexes: [u8; 20] = hexes.try_into().expect("Wrong length checking hash");

        if hexes != tf.info.get_piece_hash(p) {
            println!("piece {}; Hash doesn't match!", p);
            missing_pieces.push(p);
        } else {
            println!("piece {}; Correct hash!", p);
        }
    }
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
