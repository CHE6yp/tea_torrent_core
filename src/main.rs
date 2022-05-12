
use ureq::Response;

use bendy::decoding::Decoder;
use std::time::Duration;

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;

use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use bendy::encoding::{AsString, Error as EncodeError, SingleItemEncoder, ToBencode};

use sha1::{Digest, Sha1};

fn main() {
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x).unwrap();

    let info_hash = InfoHash::new(&x);

    if let Ok(r) = connect_to_tracker(&tf.announce, &info_hash, tf.info.length) {
        let respone = get_peers(r).unwrap();
        println!("{:?}", respone);
        connect_to_peers(respone, &info_hash);
    } else {
        println!("Connection to {} failed", &tf.announce);
        println!();
        println!("Trying backup trackers");
        for tracker in trackers() {
            println!("Trying {}", tracker);
            if let Ok(r) = connect_to_tracker(&tracker, &info_hash, tf.info.length) {
                let respone = get_peers(r).unwrap();
                println!("{:?}", respone);

                connect_to_peers(respone, &info_hash);
            }
        }
    }

    let mut file = std::fs::File::create("Ben.torrent").unwrap();
    file.write_all(&tf.info.to_bencode().unwrap()).unwrap();
    // let mut file = std::fs::File::create("Profile.torrent").unwrap();
    // file.write_all(&tf.info.profiles[0].to_bencode().unwrap());
}

fn connect_to_peers(respone: TrackerResponse, info_hash: &InfoHash) {
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
            let mut arr = vec![19];
            arr.extend(b"BitTorrent protocol");
            arr.extend([0, 0, 0, 0, 0, 0, 0, 0]);
            arr.extend(info_hash.raw());
            arr.extend(b"-TE0001-004815162342"); //12 rand numbers at the end TODO
            s.write(&arr).expect("Couldn't write buffer; Handshake"); //28

            //reading handshake
            let mut handshake_buff = [0u8; 68];
            s.read(&mut handshake_buff).expect("Couldn't read buffer; Handshake");
            println!("{:?}", handshake_buff);
            println!("{:?}", String::from_utf8_lossy(&handshake_buff));

            //reading actuall data
            loop {
                let mut message_size = [0u8; 4];
                s.read(&mut message_size).expect("Couldn't read buffer");
                let message_size = big_endian_to_u32(&message_size);
                //println!("message size {:?}", message_size);

                if message_size == 0 {
                    println!("keep alive");
                    continue;
                }

                let mut message_buf = vec![0u8; message_size as usize];
                s.read_exact(&mut message_buf).expect("Couldn't read buffer");
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
                    0 => println!("choke"),
                    1 => println!("unchoke"),
                    2 => println!("interested"),
                    3 => println!("not interested"),
                    4 => println!("have"),
                    5 => println!("bitfield"),
                    6 => println!("request"),
                    7 => println!("piece"),
                    8 => println!("cancel"),
                    9 => println!("port"),
                    _ => println!("WHAT?!"),
                }

                println!("{:?}", message_buf);
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

fn get_peers(resp: Response) -> Result<TrackerResponse, DecodeError> {
    let mut bytes: Vec<u8> = Vec::new();

    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(TrackerResponse::from_bencode(&bytes)?)
}

fn connect_to_tracker(
    announce: &str,
    info_hash: &InfoHash,
    length: usize,
) -> Result<Response, ureq::Error> {
    let url = format!("{}{}info_hash={}&port=50658&uploaded=0&downloaded=0&left={}&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1",
            announce,
            if announce.contains("?") {"&"} else {"?"},
            info_hash.as_string_url_encoded(),
            length
        );
    let _body = ureq::get(&url)
        .set("Content-Type", "application/octet-stream")
        .call();
    _body
}

#[derive(Debug)]
struct TorrentFile {
    announce: String,
    info: Info,
}

impl FromBencode for TorrentFile {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut announce = None;
        let mut info = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"announce", value) => {
                    announce = String::decode_bencode_object(value)
                        .context("announce")
                        .map(Some)?;
                }
                (b"info", value) => {
                    let i = value.try_into_dictionary().unwrap().into_raw();
                    info = Some(Info::from_bencode(i.unwrap()))
                }
                (unknown_field, _) => {
                    println!(
                        "Not done in TorrentFile -{:?}",
                        String::from_utf8_lossy(unknown_field)
                    );
                    // return Err(DecodeError::unexpected_field(String::from_utf8_lossy(
                    //     unknown_field,
                    // )));
                }
            }
        }

        let announce = announce.ok_or_else(|| DecodeError::missing_field("announce"))?;
        let info = info
            .ok_or_else(|| DecodeError::missing_field("info"))
            .unwrap()
            .unwrap();

        Ok(TorrentFile { announce, info })
    }
}

#[derive(Debug, Clone)]
struct Info {
    //file_duration: Vec<usize>, //?
    //file_media: Vec<usize>,    //?
    length: usize,
    name: String,
    piece_length: usize,
    pieces: Vec<u8>,
    //profiles: Vec<Profile>, //?
}

impl FromBencode for Info {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        // let mut file_duration = None;
        // let mut file_media = None;
        let mut length = None;
        let mut name = None;
        let mut piece_length = None;
        let mut pieces = None;
        // let mut profiles = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                // "profiles"
                // (b"file-duration", value) => {
                //     file_duration = Vec::<usize>::decode_bencode_object(value)
                //         .context("file-duration")
                //         .map(Some)?;
                // }
                // (b"file-media", value) => {
                //     file_media = Vec::<usize>::decode_bencode_object(value)
                //         .context("file-media")
                //         .map(Some)?;
                // }
                (b"length", value) => {
                    length = usize::decode_bencode_object(value)
                        .context("length")
                        .map(Some)?;
                }
                (b"name", value) => {
                    name = String::decode_bencode_object(value)
                        .context("name")
                        .map(Some)?;
                }
                (b"piece length", value) => {
                    piece_length = usize::decode_bencode_object(value)
                        .context("piece length")
                        .map(Some)?;
                }
                (b"pieces", value) => {
                    //pieces is not human readable, so we can't put it to String
                    pieces = Some(value.try_into_bytes().unwrap().to_vec());
                }
                // (b"profiles", value) => {
                //     profiles = Vec::<Profile>::decode_bencode_object(value)
                //         .context("profiles")
                //         .map(Some)?;
                // }
                (unknown_field, _) => {
                    println!(
                        "Not done in Info - {:?}",
                        String::from_utf8_lossy(unknown_field)
                    );
                }
            }
        }

        //let file_duration =
        //    file_duration.ok_or_else(|| DecodeError::missing_field("file_duration"))?;
        //let file_media = file_media.ok_or_else(|| DecodeError::missing_field("file_media"))?;
        let length = length.ok_or_else(|| DecodeError::missing_field("length"))?;
        let name = name.ok_or_else(|| DecodeError::missing_field("name"))?;
        let piece_length =
            piece_length.ok_or_else(|| DecodeError::missing_field("piece_length"))?;
        let pieces = pieces.ok_or_else(|| DecodeError::missing_field("pieces"))?;
        //let profiles = profiles.ok_or_else(|| DecodeError::missing_field("profiles"))?;

        Ok(Info {
            //file_duration,
            //file_media,
            length,
            name,
            piece_length,
            pieces,
            //profiles,
        })
    }
}

impl ToBencode for Info {
    const MAX_DEPTH: usize = 3;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), EncodeError> {
        encoder.emit_dict(|mut e| {
            //e.emit_pair(b"file-duration", &self.file_duration)?;
            //e.emit_pair(b"file-media", &self.file_media)?;
            e.emit_pair(b"length", &self.length)?;
            e.emit_pair(b"name", &self.name)?;
            e.emit_pair(b"piece length", &self.piece_length)?;
            //Clone is expensive? TODO rewrite?
            let pieces = ByteStringWrapper(self.pieces.clone());
            e.emit_pair(b"pieces", pieces)?;
            //e.emit_pair(b"profiles", &self.profiles)?;
            Ok(())
        })
    }
}

#[derive(Debug)]
struct InfoHash {
    hash: [u8; 20],
}

impl InfoHash {
    fn new(bencode: &[u8]) -> InfoHash {
        //THIS WORKS! Takes the whole info bencode without bullshit structs
        let mut decoder = Decoder::new(bencode);
        match decoder.next_object().unwrap() {
            Some(Object::Dict(mut d)) => loop {
                match d.next_pair().unwrap() {
                    Some(x) => {
                        if x.0 == b"info" {
                            let mut hasher = Sha1::new();
                            hasher.update(x.1.try_into_dictionary().unwrap().into_raw().unwrap());
                            let hexes = hasher.finalize();
                            return InfoHash {
                                hash: hexes.try_into().expect("Wrong length"),
                            };
                        }
                    }
                    None => panic!("Wrong torrent file: no Info dictionary"),
                }
            },
            _ => panic!("Wrong torrent file: not a dictionary at top level"),
        };
    }

    fn raw(&self) -> &[u8] {
        return &self.hash;
    }

    fn as_string(&self) -> String {
        let mut hash = String::new();
        for i in 0..20 {
            hash += &format!("{:02X}", &self.hash[i]);
        }
        return hash;
    }

    fn as_string_url_encoded(&self) -> String {
        let mut hash = String::new();
        for i in 0..20 {
            hash += &format!("%{:02X}", &self.hash[i]);
        }
        return hash;
    }
}

#[derive(Debug)]
struct Profile {
    acodec: String,
    height: usize,
    vcodec: String,
    width: usize,
}

impl FromBencode for Profile {
    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut acodec = None;
        let mut height = None;
        let mut vcodec = None;
        let mut width = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"acodec", value) => {
                    acodec = String::decode_bencode_object(value)
                        .context("acodec")
                        .map(Some)?;
                }
                (b"height", value) => {
                    height = usize::decode_bencode_object(value)
                        .context("height")
                        .map(Some)?;
                }
                (b"vcodec", value) => {
                    vcodec = String::decode_bencode_object(value)
                        .context("vcodec")
                        .map(Some)?;
                }
                (b"width", value) => {
                    width = usize::decode_bencode_object(value)
                        .context("width")
                        .map(Some)?;
                }
                (unknown_field, _) => {
                    println!(
                        "Not done in Profile - {:?}",
                        String::from_utf8_lossy(unknown_field)
                    );
                }
            }
        }

        let acodec = acodec.ok_or_else(|| DecodeError::missing_field("acodec"))?;
        let height = height.ok_or_else(|| DecodeError::missing_field("height"))?;
        let vcodec = vcodec.ok_or_else(|| DecodeError::missing_field("vcodec"))?;
        let width = width.ok_or_else(|| DecodeError::missing_field("width"))?;

        Ok(Profile {
            acodec,
            height,
            vcodec,
            width,
        })
    }
}

impl ToBencode for Profile {
    const MAX_DEPTH: usize = 1;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), EncodeError> {
        encoder.emit_dict(|mut e| {
            e.emit_pair(b"acodec", &self.acodec)?;
            e.emit_pair(b"height", &self.height)?;
            e.emit_pair(b"vcodec", &self.vcodec)?;
            e.emit_pair(b"width", &self.width)?;

            Ok(())
        })
    }
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

struct ByteStringWrapper(Vec<u8>);

impl ToBencode for ByteStringWrapper {
    const MAX_DEPTH: usize = 0;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), EncodeError> {
        let content = AsString(&self.0);
        encoder.emit(&content)
    }
}
