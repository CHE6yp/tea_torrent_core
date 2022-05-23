use sha1::Digest;
use sha1::Sha1;
use std::fs::OpenOptions;
use std::io::Seek;
use std::io::SeekFrom;
use ureq::Response;

use std::time::Duration;

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;

use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};

mod tf;
use crate::tf::*;

fn main() {
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x).unwrap();
    println!("\n{}\n", tf);

    //preallocate file
    let mut file = std::fs::File::create(format!("downloads/{}", &tf.info.name)).unwrap();
    file.write_all(&vec![0; tf.info.length]).unwrap();

    if let Ok(r) = connect_to_tracker(&tf) {
        let respone = get_peers(r).unwrap();
        println!(
            "Connection to {:?} complete, connecting to peers",
            &tf.announce
        );
        connect_to_peers(respone, &tf);
    } else {
        println!("Connection to {} failed", &tf.announce);
        //TODO make backup trackers work!
        /*println!();
        println!("Trying backup trackers");
        for tracker in trackers() {
            println!("Trying {}", tracker);
            // if let Ok(r) = connect_to_tracker(&tf) {
            if let Ok(r) = connect_to_tracker(&TorrentFile{announce: tracker, info: tf.info, info_hash: tf.info_hash}) {
                let respone = get_peers(r).unwrap();
                println!("Connection to {:?} complete, connecting to peers", &tracker);

                connect_to_peers(respone, &tf);
            }
        }*/
    }
}

fn connect_to_peers(respone: TrackerResponse, tf: &TorrentFile) {
    for i in (0..respone.peers.len()).step_by(6) {
        println!(
            "{}.{}.{}.{}:{}",
            respone.peers[i],
            respone.peers[i + 1],
            respone.peers[i + 2],
            respone.peers[i + 3],
            ((respone.peers[i + 4] as u16) << 8) + respone.peers[i + 5] as u16
        );
    }
    println!("=================");
    for i in (0..respone.peers.len()).step_by(6) {
        println!(
            "Trying {}.{}.{}.{}:{}",
            respone.peers[i],
            respone.peers[i + 1],
            respone.peers[i + 2],
            respone.peers[i + 3],
            ((respone.peers[i + 4] as u16) << 8) + respone.peers[i + 5] as u16
        );
        let stream = TcpStream::connect_timeout(
            &std::net::SocketAddr::from((
                [
                    respone.peers[i],
                    respone.peers[i + 1],
                    respone.peers[i + 2],
                    respone.peers[i + 3],
                ],
                ((respone.peers[i + 4] as u16) << 8) + respone.peers[i + 5] as u16,
            )),
            Duration::from_secs(2),
        );
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
            //println!("{:?}", handshake_buff);
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

            //reading actuall data
            s.set_read_timeout(Some(Duration::from_secs(15)));
            let mut peer_status = (true, false, true, false); //am_choking = 1, am_interested = 0, peer_choking = 1, peer_interested = 0

            let mut pn = 0;
            // let pl: u32 = 65536;
            // let pl: u32 = 4096;
            let mut pl: u32 = tf.info.piece_length as u32;
            let piece_count = tf.info.length / tf.info.piece_length;
            let mut block_size: u32 = 16384; //16Kb more than that doesn't work somehow, BEP 52 or something,
            let mut piece: Vec<u8> = vec![];
            let mut offset: u32 = 0;
            let mut file = OpenOptions::new()
                .write(true)
                .open(format!("downloads/{}", &tf.info.name))
                .unwrap();

            loop {
                let mut message_size = [0u8; 4];
                let package_size = s.read(&mut message_size);
                match package_size {
                    Ok(_package_size) => {} //println!("package_size {}", package_size),
                    Err(e) => {
                        println!("No package for 5 secs; {}", e);
                        if peer_status.2 == true {
                            println!("Sending unchoke and interested");
                            s.write(&[0, 0, 0, 1, 1]); //send unchoke
                            peer_status.0 = false;
                            s.write(&[0, 0, 0, 1, 2]); //send interested
                            peer_status.1 = true;
                        } else {
                            println!("Send keep-alive");
                            s.write(&[0, 0, 0, 0]);
                            //s.write(&[0, 0, 0, 13, 6, 0,0,0,1, 0,0,0,0, 0,0,0,255 ]);
                        }
                    }
                }

                let message_size = big_endian_to_u32(&message_size);
                //println!("message size {:?}", message_size);
                if message_size == 0 {
                    //println!("keep alive");
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
                        s.write(&[0, 0, 0, 1, 1]); //send unchoke
                        s.write(&[0, 0, 0, 1, 2]); //send interested
                        peer_status.0 = false;
                        peer_status.1 = true;
                    }
                    1 => {
                        println!("unchoke");
                        peer_status.2 = false;
                    }
                    2 => println!("interested"),
                    3 => println!("not interested"),
                    4 => println!("have\n {:?}", &message_buf[1..=message_size as usize - 1]),
                    5 => println!(
                        "bitfield\n {:?}",
                        &message_buf[1..=message_size as usize - 1]
                    ),
                    6 => println!("request"),
                    7 => {
                        println!(
                            "piece {} of {} ({}%), offset {}/{} ({}%)",
                            big_endian_to_u32(&message_buf[1..5].try_into().unwrap()),
                            piece_count,
                            (big_endian_to_u32(&message_buf[1..5].try_into().unwrap()) as f32
                                / ((tf.info.length as f32 / tf.info.piece_length as f32) / 100.0))
                                as u32,
                            big_endian_to_u32(&message_buf[5..9].try_into().unwrap()),
                            pl,
                            ((big_endian_to_u32(&message_buf[5..9].try_into().unwrap())
                                + block_size) as f32
                                / (pl as f32 / 100.0)) as u32
                        );
                        piece.append(&mut message_buf[9..].to_vec());

                        println!("piece.len() {:?} == piece_length {}", piece.len(), pl);

                        //std::thread::sleep(std::time::Duration::from_secs(3));
                        if piece.len() == pl.try_into().unwrap() {
                            //check hash
                            let mut hasher = Sha1::new();
                            hasher.update(&piece);
                            let hexes = hasher.finalize();
                            let hexes: [u8; 20] =
                                hexes.try_into().expect("Wrong length checking hash");
                            if hexes != tf.info.get_piece_hash(pn) {
                                println!("Hash doesn't match!");
                                offset = 0;
                                continue;
                            } else {
                                println!("Correct hash!");
                            }

                            //write_piece(&mut file, &piece, pn as u64*(tf.info.piece_length as u64));
                            println!(
                                "SEEEEEK OFFSET {:?}",
                                pn as u64 * (tf.info.piece_length as u64)
                            );
                            file.seek(SeekFrom::Start(pn as u64 * (tf.info.piece_length as u64)))
                                .expect("seek failed");
                            file.write_all(&piece).expect("write failed");
                            piece.drain(..);

                            pn += 1;
                            offset = 0;

                            if pn > piece_count {
                                break;
                            }
                        }
                    }
                    8 => println!("cancel"),
                    9 => println!("port"),
                    _ => println!("WHAT?!"),
                }

                //continue;
                //request pieces if we are unchoked

                //request 256 pieces
                // let mut pn = 0;
                // let pl = 255;
                // let mut offset = 0;

                if peer_status.2 == false && offset < pl && pn <= piece_count {
                    let be_length = (tf.info.length as u32).to_be_bytes();
                    let be_pl = pl.to_be_bytes();
                    let be_block_size = block_size.to_be_bytes();

                    println!("Requesting piece");
                    println!("length {:?}", tf.info.length);
                    println!("length BE {:?}", be_length);
                    println!("piece_length {:?}", pl);
                    println!("piece_length BE {:?}", be_pl);
                    println!("piece_fraction {:?}", block_size);
                    println!("piece_fraction BE {:?}", be_block_size);

                    let mut request_message = vec![0, 0, 0, 13, 6]; //constant part
                    request_message.append(&mut vec![0, 0, 0, pn as u8]); //piece number TODO
                    let be_offset = offset.to_be_bytes();
                    request_message.append(&mut be_offset.to_vec()); //piece uhh, offset?

                    if pn == piece_count && offset == 0 {
                        pl = (tf.info.length - ((piece_count) * pl as usize)) as u32;
                        //bitwise magic! this finds the rightmost bit it last_piece_size
                        block_size = pl & (!(pl - 1));

                        //block_size = 1u32;
                        //be_block_size = block_size.to_be_bytes();
                    }
                    request_message.append(&mut block_size.to_be_bytes().to_vec()); //piece length
                    println!("Requesting {:?}\n", &request_message);
                    s.write(&request_message);
                    offset += block_size;
                }
            }
        } else {
            println!("Failed!");
        }
    }
}

fn big_endian_to_u32(value: &[u8; 4]) -> u32 {
    ((value[0] as u32) << 24)
        + ((value[1] as u32) << 16)
        + ((value[2] as u32) << 8)
        + value[3] as u32
}

//BACKUP TRACKERS!!!
fn trackers() -> [String; 25] {
    [
        "https://tr.abiir.top:443/announce".to_string(), //best amount of peers!!!
        "http://tracker.files.fm:6969/announce".to_string(),
        "http://tracker.mywaifu.best:6969/announce".to_string(),
        "https://tracker.nanoha.org:443/announce".to_string(),
        "http://tracker2.ctix.cn:6969/announce".to_string(),
        "http://t.overflow.biz:6969/announce".to_string(),
        "https://tracker.babico.name.tr:443/announce".to_string(),
        "http://bt.okmp3.ru:2710/announce".to_string(),
        "https://track.plop.pm:8989/announce".to_string(), //?? gave localhost as peer
        "http://tracker.openbittorrent.com:80/announce".to_string(),
        //failed
        "https://tracker.lilithraws.cf:443/announce".to_string(),
        "http://ipv6.govt.hu:6969/announce".to_string(),
        "http://open.acgnxtracker.com:80/announce".to_string(),
        "http://t.publictracker.xyz:6969/announce".to_string(),
        "https://tr.burnabyhighstar.com:443/announce".to_string(),
        "http://ipv6.1337.cx:6969/announce".to_string(),
        "http://i-p-v-6.tk:6969/announce".to_string(),
        "http://tracker.ipv6tracker.ru:80/announce".to_string(),
        "http://tracker.k.vu:6969/announce".to_string(),
        "http://t.nyaatracker.com:80/announce".to_string(),
        "http://t.acg.rip:6699/announce".to_string(),
        "https://tracker.iriseden.fr:443/announce".to_string(),
        "http://tracker.gbitt.info:80/announce".to_string(),
        "https://chihaya-heroku.120181311.xyz:443/announce".to_string(),
        "https://opentracker.i2p.rocks:443/announce".to_string(),
    ]
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

fn get_peers(resp: Response) -> Result<TrackerResponse, DecodeError> {
    let mut bytes: Vec<u8> = Vec::new();

    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(TrackerResponse::from_bencode(&bytes)?)
}

fn connect_to_tracker(tf: &TorrentFile) -> Result<Response, ureq::Error> {
    let url = format!("{}{}info_hash={}&port=50658&uploaded=0&downloaded=0&left={}&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1",
            tf.announce,
            if tf.announce.contains("?") {"&"} else {"?"},
            tf.info_hash.as_string_url_encoded(),
            tf.info.length
        );
    let _body = ureq::get(&url)
        .set("Content-Type", "application/octet-stream")
        .call();
    _body
}

#[derive(Debug)]
struct TrackerResponse {
    interval: usize,
    min_interval: usize,
    peers: Vec<u8>,
}

impl FromBencode for TrackerResponse {
    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut interval = None;
        let mut min_interval = None;
        let mut peers = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"interval", value) => {
                    interval = usize::decode_bencode_object(value)
                        .context("interval")
                        .map(Some)?;
                }
                (b"min interval", value) => {
                    min_interval = usize::decode_bencode_object(value)
                        .context("min interval")
                        .map(Some)?;
                }
                (b"peers", value) => {
                    //peers is not human readable, so we can't put it to String
                    peers = Some(value.try_into_bytes().unwrap().to_vec());
                }
                (unknown_field, _) => {
                    println!(
                        "Not done in TrackerResponse - {:?}",
                        String::from_utf8_lossy(unknown_field)
                    );
                }
            }
        }

        let interval = interval.ok_or_else(|| DecodeError::missing_field("interval"))?;
        let min_interval =
            min_interval.ok_or_else(|| DecodeError::missing_field("min_interval"))?;
        let peers = peers.ok_or_else(|| DecodeError::missing_field("peers"))?;

        Ok(TrackerResponse {
            interval,
            min_interval,
            peers,
        })
    }
}
