use bendy::decoding::Decoder;
use std::error::Error;

use std::env;
use std::fs;
use std::io::{Read, Write};

use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use bendy::encoding::{AsString, Error as EncodeError, SingleItemEncoder, ToBencode};

use sha1::{Digest, Sha1};

fn main() {
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x).unwrap();
    //println!("{:?}", tf);

    let hash_string = get_info_hash_encoded(&x);
    println!("{:?}", hash_string);

    let mut bytes: Vec<u8> = Vec::new();
    // let body = ureq::get("http://bt4.t-ru.org/ann?pk=81d1bedc2cf1de678b0887c7619f6d45&info_hash=%2awMG%81C%9d%a5%8e%a2%11%0bEsM%8f%c5%80%cd%cd&port=50658&uploaded=0&downloaded=0&left=1566951424&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1")
    let url = format!("{}&info_hash={}&port=50658&uploaded=0&downloaded=0&left={}&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1",
            tf.announce,
            hash_string,
            tf.info.length
        );
    let _body = ureq::get(&url)
        .set("Content-Type", "application/octet-stream")
        .call()
        .unwrap()
        .into_reader()
        .read_to_end(&mut bytes);
    let respone = TrackerResponse::from_bencode(&bytes).unwrap();
    println!("{:?}", respone);
    println!("{:?}", &respone.peers.len());
    for i in (0..respone.peers.len()).step_by(6) {
        println!(
            "{}.{}.{}.{}:{}",
            respone.peers[i],
            respone.peers[i + 1],
            respone.peers[i + 2],
            respone.peers[i + 3],
            ((respone.peers[i + 4] as u32) << 8) + respone.peers[i + 5] as u32
        );
    }

    // let mut file = std::fs::File::create("Ben.torrent").unwrap();
    // file.write_all(&tf.info.to_bencode().unwrap());
    // let mut file = std::fs::File::create("Profile.torrent").unwrap();
    // file.write_all(&tf.info.profiles[0].to_bencode().unwrap());
}

fn get_info_hash_encoded(bencode: &[u8]) -> String {
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
                        let mut hash = String::new();
                        for i in 0..20 {
                            hash += &format!("%{:02X}", &hexes[i]);
                        }
                        return hash;
                    }
                }
                None => panic!("Wrong torrent file: no Info dictionary"),
            }
        },
        _ => panic!("Wrong torrent file: not a dictionary at top level"),
    };
}

#[derive(Debug)]
struct TorrentFile {
    announce: String,
    info: Info,
}

#[derive(Debug)]
struct Info {
    //file_duration: Vec<usize>, //?
    //file_media: Vec<usize>,    //?
    length: usize,
    name: String,
    piece_length: usize,
    pieces: Vec<u8>,
    //profiles: Vec<Profile>, //?
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

impl FromBencode for Info {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut file_duration = None;
        let mut file_media = None;
        let mut length = None;
        let mut name = None;
        let mut piece_length = None;
        let mut pieces = None;
        let mut profiles = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                // "profiles"
                (b"file-duration", value) => {
                    file_duration = Vec::<usize>::decode_bencode_object(value)
                        .context("file-duration")
                        .map(Some)?;
                }
                (b"file-media", value) => {
                    file_media = Vec::<usize>::decode_bencode_object(value)
                        .context("file-media")
                        .map(Some)?;
                }
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
                (b"profiles", value) => {
                    profiles = Vec::<Profile>::decode_bencode_object(value)
                        .context("profiles")
                        .map(Some)?;
                }
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
