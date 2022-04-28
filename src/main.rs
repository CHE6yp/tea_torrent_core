use bendy::decoding::{Error, ResultExt, FromBencode, Object};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x);
    println!("{:?}", tf);
}

#[derive(Debug)]
struct TorrentFile {
    announce: String,
    info: Info,
}

#[derive(Debug)]
struct Info {
    length: usize,
    name: String,
    piece_length: usize,
    pieces: Vec<u8>,
}

impl FromBencode for TorrentFile {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, Error> {
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
                    println!("{:?}", String::from_utf8_lossy(unknown_field));
                    // return Err(Error::unexpected_field(String::from_utf8_lossy(
                    //     unknown_field,
                    // )));
                }
            }
        }

        let announce = announce.ok_or_else(|| Error::missing_field("announce"))?;
        let info = info
            .ok_or_else(|| Error::missing_field("info"))
            .unwrap()
            .unwrap();

        Ok(TorrentFile { announce, info })
    }
}

impl FromBencode for Info {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, Error> {
        let mut length = None;
        let mut name = None;
        let mut piece_length = None;
        let mut pieces = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
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
                (unknown_field, _) => {
                    println!("{:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let length = length.ok_or_else(|| Error::missing_field("length"))?;
        let name = name.ok_or_else(|| Error::missing_field("name"))?;
        let piece_length = piece_length.ok_or_else(|| Error::missing_field("piece_length"))?;
        let pieces = pieces.ok_or_else(|| Error::missing_field("pieces"))?;

        Ok(Info {
            length,
            name,
            piece_length,
            pieces,
        })
    }
}
