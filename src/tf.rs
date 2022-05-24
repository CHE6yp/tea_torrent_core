use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use bendy::encoding::{AsString, Error as EncodeError, SingleItemEncoder, ToBencode};
use sha1::{Digest, Sha1};
use std::fmt;

#[derive(Debug)]
pub struct TorrentFile {
    pub announce: String,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub info: Info,
    pub info_hash: InfoHash,
}

impl FromBencode for TorrentFile {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut announce = None;
        let mut announce_list = None;
        let mut info = None;
        let mut info_hash = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"announce", value) => {
                    announce = String::decode_bencode_object(value)
                        .context("announce")
                        .map(Some)?;
                }
                (b"announce-list", value) => {
                    announce_list = Vec::<Vec<String>>::decode_bencode_object(value)
                        .context("announce-list")
                        .map(Some)?;
                }
                (b"info", value) => {
                    let i = value.try_into_dictionary().unwrap().into_raw().unwrap();

                    info_hash = Some(InfoHash::new(i));
                    info = Some(Info::from_bencode(i))
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
        let info = info.ok_or_else(|| DecodeError::missing_field("info"))??;
        let info_hash = info_hash.ok_or_else(|| DecodeError::missing_field("info_hash"))?;

        Ok(TorrentFile {
            announce,
            announce_list,
            info,
            info_hash,
        })
    }
}

impl fmt::Display for TorrentFile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "\x1b[1mTorrent File\x1b[0m\nannounce: {}\ninfo_hash: {}\ninfo: {}\nannounce_list {:?}",
            self.announce,
            self.info_hash.as_string(),
            self.info,
            self.announce_list,
        )
    }
}

#[derive(Debug, Clone)]
pub struct Info {
    //file_duration: Vec<usize>, //?
    //file_media: Vec<usize>,    //?
    pub length: usize,
    pub name: String,
    pub piece_length: u32,
    pieces: Vec<u8>,
    //profiles: Vec<Profile>, //?
}

impl Info {
    pub fn get_piece_hash(&self, piece: usize) -> &[u8] {
        if piece > self.pieces.len() / 20 {
            panic!("PIECES ARRAY OVERFLOW!");
        }

        &self.pieces[piece * 20..(piece + 1) * 20]
    }
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
                    piece_length = u32::decode_bencode_object(value)
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

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let piece_count = self.pieces.len() / 20;
        let mut pieces_str = String::new();
        for i in (0..self.pieces.len()).step_by(20) {
            pieces_str = format!("{}{:?}\n", pieces_str, &self.pieces[i..(i + 20)])
        }
        write!(
            f,
            "length: {}\nname: {}\npieces count: {}, piece length: {}",
            self.length, self.name, piece_count, self.piece_length,
        )
    }
}

#[derive(Debug)]
pub struct InfoHash {
    hash: [u8; 20],
}

impl InfoHash {
    fn new(bencode: &[u8]) -> InfoHash {
        let mut hasher = Sha1::new();
        hasher.update(bencode);
        let hexes = hasher.finalize();
        return InfoHash {
            hash: hexes.try_into().expect("Wrong length"),
        };
    }

    pub fn raw(&self) -> &[u8] {
        return &self.hash;
    }

    pub fn as_string(&self) -> String {
        let mut hash = String::new();
        for i in 0..20 {
            hash += &format!("{:02X}", &self.hash[i]);
        }
        return hash;
    }

    pub fn as_string_url_encoded(&self) -> String {
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

struct ByteStringWrapper(Vec<u8>);

impl ToBencode for ByteStringWrapper {
    const MAX_DEPTH: usize = 0;

    fn encode(&self, encoder: SingleItemEncoder) -> Result<(), EncodeError> {
        let content = AsString(&self.0);
        encoder.emit(&content)
    }
}
