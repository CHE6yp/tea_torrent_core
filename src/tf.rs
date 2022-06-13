use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use bendy::encoding::{AsString, Error as EncodeError, SingleItemEncoder, ToBencode};
use sha1::{Digest, Sha1};
use std::fmt;

#[derive(Debug, Clone)]
pub struct TorrentFile {
    pub announce: String,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub info: Info,
    pub info_hash: InfoHash,
    pub creation_date: Option<u32>,
    pub comment: Option<String>,
    pub created: Option<String>,
    pub encoding: Option<String>,
}

impl FromBencode for TorrentFile {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut announce = None;
        let mut announce_list = None;
        let mut info = None;
        let mut info_hash = None;
        let mut creation_date = None;
        let mut comment = None;
        let mut created = None;
        let mut encoding = None;

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
                (b"creation_date", value) => {
                    creation_date = u32::decode_bencode_object(value)
                        .context("creation_date")
                        .map(Some)?;
                }
                (b"comment", value) => {
                    comment = String::decode_bencode_object(value)
                        .context("comment")
                        .map(Some)?;
                }
                (b"created", value) => {
                    created = String::decode_bencode_object(value)
                        .context("created")
                        .map(Some)?;
                }
                (b"encoding", value) => {
                    encoding = String::decode_bencode_object(value)
                        .context("encoding")
                        .map(Some)?;
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
            creation_date,
            comment,
            created,
            encoding,
        })
    }
}

impl fmt::Display for TorrentFile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}info_hash: {}", self.info, self.info_hash.as_string(),)
    }
}

#[derive(Debug, Clone)]
pub struct Info {
    //file_duration: Vec<usize>, //?
    //file_media: Vec<usize>,    //?
    pub length: usize,
    pub name: String,
    pub piece_length: u32,
    pub piece_count: u32,
    pieces: Vec<u8>,
    pub files: Vec<File>,
    //profiles: Vec<Profile>, //?
}

impl Info {
    pub fn get_piece_hash(&self, piece: usize) -> &[u8] {
        if piece > self.pieces.len() / 20 {
            panic!("PIECES ARRAY OVERFLOW!");
        }
        &self.pieces[piece * 20..(piece + 1) * 20]
    }

    pub fn get_last_piece_size(&self) -> u32 {
        self.length as u32 - (self.piece_length * (self.piece_count - 1))
    }
}

impl FromBencode for Info {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        // let mut file_duration = None;
        // let mut file_media = None;
        let mut len = None;
        let mut name = None;
        let mut piece_length = None;
        let mut pieces = None;
        let mut files = vec![];
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
                    len = usize::decode_bencode_object(value)
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
                    let value = value.try_into_bytes().unwrap().to_vec();
                    if value.len() % 20 == 0 {
                        pieces = Some(value);
                    }
                }
                (b"files", value) => {
                    let mut value = value.try_into_list().unwrap();
                    while let Some(d) = value.next_object()? {
                        let d = d.try_into_dictionary().unwrap().into_raw().unwrap();

                        let file = File::from_bencode(d)?;
                        files.push(file);
                    }
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
        let name = name.ok_or_else(|| DecodeError::missing_field("name"))?;
        let piece_length =
            piece_length.ok_or_else(|| DecodeError::missing_field("piece_length"))?;
        let pieces = pieces.ok_or_else(|| DecodeError::missing_field("pieces"))?;
        let mut length = 0;
        if len == None && !files.is_empty() {
            for f in &files {
                length += f.length
            }
        } else {
            length = len.ok_or_else(|| DecodeError::missing_field("length"))?;
        }
        if files.is_empty() {
            files.push(File {
                length,
                path: name.clone(),
            });
        }
        //let profiles = profiles.ok_or_else(|| DecodeError::missing_field("profiles"))?;

        let piece_count = (length / piece_length as usize
            + if length % piece_length as usize == 0 {
                0
            } else {
                1
            }) as u32;

        Ok(Info {
            //file_duration,
            //file_media,
            length,
            name,
            piece_length,
            pieces,
            files,
            //profiles,
            piece_count,
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
        let mut files = String::new();
        for f in &self.files {
            files += &format!(" - {} \x1b[1m{}\x1b[0m\n", f.path, f.length)
        }
        write!(
            f,
            "\x1b[1m{}\x1b[0m\nLength: \x1b[1m{}\x1b[0m\nFiles:\n{}",
            self.name, self.length, files
        )
    }
}

#[derive(Debug, Clone)]
pub struct InfoHash {
    hash: [u8; 20],
}

impl InfoHash {
    fn new(bencode: &[u8]) -> InfoHash {
        let mut hasher = Sha1::new();
        hasher.update(bencode);
        let hexes = hasher.finalize();
        InfoHash {
            hash: hexes.try_into().expect("Wrong length"),
        }
    }

    pub fn raw(&self) -> &[u8] {
        &self.hash
    }

    pub fn as_string(&self) -> String {
        let mut hash = String::new();
        for i in 0..20 {
            hash += &format!("{:02X}", &self.hash[i]);
        }
        hash
    }

    pub fn as_string_url_encoded(&self) -> String {
        let mut hash = String::new();
        for i in 0..20 {
            hash += &format!("%{:02X}", &self.hash[i]);
        }
        hash
    }
}

#[derive(Debug, Clone)]
pub struct File {
    pub length: usize,
    pub path: String,
}

impl FromBencode for File {
    //const EXPECTED_RECURSION_DEPTH: usize = 1;

    fn decode_bencode_object(object: Object) -> Result<Self, DecodeError> {
        let mut length = None;
        let mut path = None;

        let mut dict = object.try_into_dictionary()?;
        while let Some(pair) = dict.next_pair()? {
            match pair {
                (b"length", value) => {
                    length = usize::decode_bencode_object(value)
                        .context("length")
                        .map(Some)?;
                }
                (b"path", value) => {
                    let p = Vec::<String>::decode_bencode_object(value).context("path")?;
                    let pb = p.join("/");
                    path = Some(pb);
                }
                (unknown_field, _) => {
                    println!(
                        "Not done in File -{:?}",
                        String::from_utf8_lossy(unknown_field)
                    );
                }
            }
        }

        let length = length.ok_or_else(|| DecodeError::missing_field("length"))?;
        let path = path.ok_or_else(|| DecodeError::missing_field("path"))?;

        Ok(File { length, path })
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
