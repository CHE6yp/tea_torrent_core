use bendy::decoding::FromBencode;
use rand::Rng;

use std::fs;
use std::io::{stdout, ErrorKind, Read, Result, Write};
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
pub mod content;
use content::*;

const BLOCK_SIZE: u32 = 16384;

pub struct Torrent {
    pub content: Content,
    pub torrent_file: TorrentFile,
}

impl Torrent {
    pub fn new(
        torrent_file_path: String,
        download_folder: Option<String>,
        content_events: Option<ContentEvents>,
    ) -> Torrent {
        let tf_raw = fs::read(&torrent_file_path).unwrap();
        let tf = TorrentFile::from_bencode(&tf_raw).unwrap();
        println!("{}", tf);
        println!();

        let mut content = Content::new(&tf, download_folder);

        // content.events.preallocaion_end.push(Box::new(name));
        // content.events.hash_checked.push(Box::new(|x,y| { println!("{:?}/{}",x,y );}));
        // content.events.hash_checked.push(Box::new(|x,y| { println!("Have {} out of {}",x,y );}));
        if let Some(events) = content_events {
            content.events = events;
        }
        Torrent {
            torrent_file: tf,
            content,
        }
    }

    pub fn run(&self) {
        let content = Arc::new(&self.content);
        content.preallocate();
        content.check_content_hash();
        println!("{:?}", content.get_bitfield());

        let r = connect_to_tracker(&self.torrent_file, false);
        if r.is_none() {
            println!("Connection failed");
            return;
        }

        let respone = r.unwrap();
        println!("Connection complete, connecting to peers");

        let mut peers = connect_to_peers(
            respone,
            Handshake::new(self.torrent_file.info_hash.raw()),
            self.torrent_file.info.piece_count as usize,
        );
        let mut handles: Vec<thread::JoinHandle<_>> = vec![];

        let (tx, rx) = channel();

        //recieving messages from peers
        for peer in &peers {
            let peer = Arc::clone(peer);
            let tx = tx.clone();
            let join_handle = thread::Builder::new()
                .name(peer.id_string())
                .spawn(move || loop {
                    let message = peer.get_message();

                    match message {
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => {
                            panic!("Couldn't read buffer; {:?}", e.kind(),);
                        }
                        Ok(message) => match message {
                            PeerMessage::KeepAlive => (),
                            PeerMessage::Choke => {
                                println!("Choked by {}", peer.id_string());
                                peer.status.lock().unwrap().2 = true;
                            }
                            PeerMessage::Unchoke => {
                                println!("Unchoked by {}", peer.id_string());
                                peer.status.lock().unwrap().2 = false;
                                *peer.busy.lock().unwrap() = false;
                            }
                            PeerMessage::Interested => {
                                println!("interested");
                                peer.status.lock().unwrap().3 = true;
                            }
                            PeerMessage::NotInterested => {
                                println!("not interested");
                                peer.status.lock().unwrap().3 = false;
                            }
                            PeerMessage::Have(index) => {
                                peer.add_piece_to_bitfield(index);
                            }
                            PeerMessage::Bitfield(field) => {
                                *peer.bitfield.lock().unwrap() = field;
                            }
                            PeerMessage::Request(_index, _begin, _length) => {
                                println!("request");
                            }
                            PeerMessage::Piece(index, begin, block) => {
                                tx.send((Arc::clone(&peer), (index, begin, block))).unwrap();
                            }
                            PeerMessage::Cancel(_index, _begin, _length) => {
                                println!("cancel");
                            }
                            PeerMessage::Port(_port) => {
                                println!("port {}", peer.id_string());
                            }
                        },
                    }
                })
                .unwrap();

            handles.push(join_handle);
        }

        let tf = Arc::new(&self.torrent_file);
        //recieving blocks and writing them to pieces (and then to file)
        let content_write = Arc::clone(&content); //THIS is why SELF ESCAPES in an unscoped thread!!!!
        thread::scope(|s| {
            s.spawn(move || {
                rx.iter().for_each(|(peer, (index, begin, block))| {
                    let piece_number = index;
                    let offset = begin;

                    //TODO need to find a way to make peer not busy before writing the piece to file.
                    //my theory is we will be able to download and write at the same time then
                    //and this thread will make sense.
                    //It will probably take more RAM though
                    let r = content_write.add_block(piece_number as usize, offset as usize, &block);
                    match r {
                        Some(true) => {
                            println!(
                                " \x1b[92mCorrect hash!\x1b[0m Piece {} from {}",
                                piece_number,
                                peer.id_string(),
                            );
                            *peer.busy.lock().unwrap() = false;
                        }
                        Some(false) => {
                            println!(
                                " \x1b[91mHash doesn't match!\x1b[0m Piece {} from{}",
                                piece_number,
                                peer.id_string(),
                            );
                            *peer.busy.lock().unwrap() = false;
                        }
                        None => (),
                    }
                });
                println!("Write thread DONE!");
            });

            //sending messages to peers
            // let mut missing_pieces = content.missing_pieces.iter();
            // let mut piece = missing_pieces.next();

            loop {
                let piece_o = content
                    .pieces
                    .iter()
                    .find(|piece| piece.lock().unwrap().status == PieceStatus::Missing);
                let &mut piece;
                match piece_o {
                    Some(p) => piece = p,
                    None => continue,
                }
                let p = piece.lock().unwrap().number;

                let peersclone = peers.clone();
                let peersclone = peersclone
                    .into_iter()
                    .filter(|peer| {
                        peer.has_piece(p.try_into().unwrap()) && !(*peer.busy.lock().unwrap())
                    })
                    .collect::<Vec<Arc<Peer>>>();
                let peer = peersclone.first();

                if peer.is_none() {
                    continue;
                }

                let peer = Arc::clone(peer.unwrap());

                let piece_length = if p == tf.info.piece_count - 1 {
                    //last piece
                    tf.info.length as u32 - (tf.info.piece_length as u32 * p)
                } else {
                    tf.info.piece_length
                };
                let res = peer.request(&peer.stream, p as u32, piece_length);
                match res {
                    Ok(true) => piece.lock().unwrap().make_awaiting(),
                    Ok(false) => (),
                    Err(_e) => {
                        println!("\x1b[91mRemoving peer {} \x1b[0m", peer.id_string());
                        let index = peersclone.iter().position(|x| x.id == peer.id).unwrap();
                        peers.remove(index);
                    }
                }
            }
        });
        //println!("missing_pieces DONE!");

        // for handle in handles {
        //     let _r = handle.join();
        // }
    }
}

fn connect_to_peers(
    respone: TrackerResponse,
    handshake: Handshake,
    piece_count: usize,
) -> Vec<Arc<Peer>> {
    let mut streams = vec![];
    let pool = ThreadPool::new(9);
    let (tx, rx) = channel();
    let respone = Arc::new(respone);

    enum Result {
        Done(Peer),
        Error,
        InvalidHash,
    }

    for i in 0..respone.peers.len() {
        let respone = Arc::clone(&respone);
        let tx = tx.clone();

        pool.execute(move || {
            stdout().flush().unwrap();
            let stream = TcpStream::connect_timeout(&respone.peers[i], Duration::from_secs(2));

            if let Ok(mut s) = stream {
                //s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
                //writing handshake
                let _r = s
                    .write(&handshake.raw)
                    .expect("Couldn't write buffer; Handshake");
                //reading handshake
                let mut peer_handshake = [0u8; 68];
                let _r = s.read_exact(&mut peer_handshake);

                if let Err(_e) = _r {
                    tx.send((Result::Error, respone.peers[i]))
                        .expect("channel will be there waiting for the pool");
                    return;
                }
                if peer_handshake[28..48] != handshake.raw[28..48] {
                    tx.send((Result::InvalidHash, respone.peers[i]))
                        .expect("channel will be there waiting for the pool");
                    return;
                }
                let mut peer_id = [0; 20];
                peer_id.clone_from_slice(&peer_handshake[48..68]);
                // s.set_nonblocking(true);
                tx.send((
                    Result::Done(Peer {
                        id: peer_id,
                        stream: s,
                        bitfield: Mutex::new(vec![0; piece_count]),
                        status: Mutex::new((true, false, true, false)),
                        busy: Mutex::new(false),
                    }),
                    respone.peers[i],
                ))
                .expect("channel will be there waiting for the pool");
            } else {
                tx.send((Result::Error, respone.peers[i]))
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
            Result::Error => {
                println!("\x1b[91mFailed!\x1b[0m");
            }
            Result::InvalidHash => {
                println!("\x1b[91mInvalid info hash!\x1b[0m");
            }
        }
    });
    streams
}

#[derive(Debug)]
struct Handshake {
    raw: [u8; 68],
}

impl Handshake {
    fn new(info_hash: &[u8; 20]) -> Handshake {
        let mut arr = vec![19];
        arr.extend(b"BitTorrent protocol");
        arr.extend([0, 0, 0, 0, 0, 0, 0, 0]);
        arr.extend(info_hash);
        arr.extend(b"-tT0030-");
        let random_id: [u8; 12] = (0..12)
            .map(|_| rand::thread_rng().gen_range(48..58))
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        arr.extend(&random_id);
        let raw = arr.try_into().unwrap();
        Handshake { raw }
    }
}

#[derive(Debug)]
pub struct Peer {
    id: [u8; 20],
    stream: TcpStream,
    bitfield: Mutex<Vec<u8>>,
    //am_choking = 1, am_interested = 0, peer_choking = 1, peer_interested = 0
    status: Mutex<(bool, bool, bool, bool)>,
    busy: Mutex<bool>,
}

impl Peer {
    fn get_message(&self) -> Result<PeerMessage> {
        let mut message_size = [0u8; 4];
        let mut stream = &self.stream;
        let package_size = stream.read_exact(&mut message_size);
        match package_size {
            Ok(_package_size) => {}
            Err(e) => {
                println!("No package for some secs; read timeout; {}", e);

                println!("Send keep-alive");
                let r = stream.write(&[0, 0, 0, 0]);
                match r {
                    Err(e) if e.kind() == ErrorKind::Interrupted => return Err(e),
                    Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                        println!("\x1b[91mConnection Reset\x1b[0m {}", self.id_string());
                        panic!("{:?}", e.kind());
                    }
                    Err(e) if e.kind() == ErrorKind::ConnectionAborted => {
                        println!("\x1b[91mConnection aborted\x1b[0m {}", self.id_string());
                        panic!("{:?}", e.kind());
                    }
                    Err(e) => println!("Error writing buffer: {:?}", e),
                    _ => (),
                }
            }
        }
        let message_size = u32::from_be_bytes(message_size);
        if message_size == 0 {
            return Ok(PeerMessage::KeepAlive);
        }

        let mut message_buf = vec![0u8; message_size as usize];
        stream.read_exact(&mut message_buf)?;
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
        let message = match &message_buf[0] {
            0 => PeerMessage::Choke,
            1 => PeerMessage::Unchoke,
            2 => PeerMessage::Interested,
            3 => PeerMessage::NotInterested,
            4 => PeerMessage::Have(u32::from_be_bytes(message_buf[1..5].try_into().unwrap())),
            5 => PeerMessage::Bitfield(message_buf[1..].to_vec()),
            6 => PeerMessage::Request(
                u32::from_be_bytes(message_buf[1..5].try_into().unwrap()),
                u32::from_be_bytes(message_buf[5..9].try_into().unwrap()),
                u32::from_be_bytes(message_buf[9..].try_into().unwrap()),
            ),
            7 => PeerMessage::Piece(
                u32::from_be_bytes(message_buf[1..5].try_into().unwrap()),
                u32::from_be_bytes(message_buf[5..9].try_into().unwrap()),
                message_buf[9..].to_vec(),
            ),
            8 => PeerMessage::Cancel(
                u32::from_be_bytes(message_buf[1..5].try_into().unwrap()),
                u32::from_be_bytes(message_buf[5..9].try_into().unwrap()),
                u32::from_be_bytes(message_buf[9..13].try_into().unwrap()),
            ),
            9 => PeerMessage::Port(u32::from_be_bytes(message_buf[1..5].try_into().unwrap())),
            _ => {
                panic!("Unknown message!");
            }
        };
        Ok(message)
    }

    fn request(
        &self,
        mut stream: &TcpStream,
        piece_number: u32,
        piece_length: u32,
    ) -> Result<bool> {
        if let Ok(mut st) = self.status.lock() {
            if !st.1 && st.2 {
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
            // println!("Still choked");
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
#[derive(Debug)]
enum PeerMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request(u32, u32, u32),
    Piece(u32, u32, Vec<u8>),
    Cancel(u32, u32, u32),
    Port(u32),
}
