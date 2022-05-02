use std::io::{Read, Write};
use std::env;
use std::fs;

use bendy::decoding::{Error as DecodeError, ResultExt, FromBencode, Object};
use bendy::encoding::{Error as EncodeError, AsString, ToBencode, SingleItemEncoder};



fn main() {
    let args: Vec<String> = env::args().collect();
    let x = fs::read(&args[1]).unwrap();

    let tf = TorrentFile::from_bencode(&x).unwrap();
    println!("{:?}", tf);


    //plain tcp try
    /*println!("{:?}", "catfact.ninja:8000".to_socket_addrs().unwrap());
    let mut stream = TcpStream::connect("catfact.ninja:8000").unwrap();
    let _res = stream.write(b"GET /fact HTTP/1.1\r\nUserAgent: poop\r\nConnection: close\r\n\r\n");

    let mut buf = String::new();
    let result = stream.read_to_string(&mut buf).unwrap();
    println!("result = {}", result);
    println!("buf = {}", buf);*/


    //ureq try
    /*let body: String = ureq::get("https://catfact.ninja/fact")
        .set("Example-Header", "header value")
        .call().unwrap()
        .into_string().unwrap();

    let body: String = ureq::get(&(tf.announce+"ddddddd"))
        .set("Example-Header", "header value")
        .call().unwrap()
        .into_string().unwrap();*/


	let mut bytes: Vec<u8> = Vec::new();
    let body = ureq::get("http://bt4.t-ru.org/ann?pk=81d1bedc2cf1de678b0887c7619f6d45&info_hash=%2awMG%81C%9d%a5%8e%a2%11%0bEsM%8f%c5%80%cd%cd&port=50658&uploaded=0&downloaded=0&left=1566951424&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1")
        .set("Content-Type", "application/octet-stream")
        .call().unwrap()
        //.into_string().unwrap();
		.into_reader()
	    .read_to_end(&mut bytes);

    //println!("{:?}", body);
    let respone = TrackerResponse::from_bencode(&bytes).unwrap();
    //println!("{:?}", respone);
    //println!("{:?}", String::from_utf8_lossy(&x));
    //println!("{:?}", String::from_utf8_lossy(&tf.info.to_bencode().unwrap()));

    let mut file = std::fs::File::create("Ben.torrent").unwrap();
	file.write_all(&tf.info.to_bencode().unwrap());

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

#[derive(Debug)]
struct TrackerResponse {
	interval: usize,
	min_interval: usize,
	peers: Vec<u8>
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
                    println!("Not done in TorrentFile -{:?}", String::from_utf8_lossy(unknown_field));
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
        let mut length = None;
        let mut name = None;
        let mut piece_length = None;
        let mut pieces = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
    			//"file-duration"
				// "file-media"
				// "profiles"
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
                    println!("Not done in Info - {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let length = length.ok_or_else(|| DecodeError::missing_field("length"))?;
        let name = name.ok_or_else(|| DecodeError::missing_field("name"))?;
        let piece_length = piece_length.ok_or_else(|| DecodeError::missing_field("piece_length"))?;
        let pieces = pieces.ok_or_else(|| DecodeError::missing_field("pieces"))?;

        Ok(Info {
            length,
            name,
            piece_length,
            pieces,
        })
    }
}

impl ToBencode for Info {
    const MAX_DEPTH: usize = 2;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), EncodeError> {
        encoder.emit_dict(|mut e| {
            e.emit_pair(b"length", &self.length)?;
			e.emit_pair(b"name", &self.name)?;
			e.emit_pair(b"piece length", &self.piece_length)?;
			//Clone is expensive? TODO rewrite?
			let pieces = ByteStringWrapper(self.pieces.clone());
			e.emit_pair(b"pieces", pieces)?;

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
                    println!("Not done in TrackerResponse - {:?}", String::from_utf8_lossy(unknown_field));
                }
            }
        }

        let interval = interval.ok_or_else(|| DecodeError::missing_field("interval"))?;
        let min_interval = min_interval.ok_or_else(|| DecodeError::missing_field("min_interval"))?;
        let peers = peers.ok_or_else(|| DecodeError::missing_field("peers"))?;

        Ok(TrackerResponse {
            interval,
            min_interval,
            peers
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