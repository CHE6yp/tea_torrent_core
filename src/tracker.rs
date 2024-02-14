use crate::{Content, PieceStatus};
use crate::TorrentFile;
use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use std::net::SocketAddr;
mod http;

#[derive(Debug)]
pub struct TrackerResponse {
    #[allow(dead_code)]
    pub interval: usize,
    #[allow(dead_code)]
    pub min_interval: usize,
    pub peers: Vec<SocketAddr>,
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
                    let p = value.try_into_bytes().unwrap().to_vec();
                    let mut v = vec![];
                    for i in (0..p.len()).step_by(6) {
                        v.push(SocketAddr::from((
                            [p[i], p[i + 1], p[i + 2], p[i + 3]],
                            ((p[i + 4] as u16) << 8) + p[i + 5] as u16,
                        )));
                    }
                    peers = Some(v);
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

pub fn connect_to_tracker(tf: &TorrentFile, content: &Content) -> Option<TrackerResponse> {
    let conn = |tracker: &str, info_hash: &str| -> (u16, Vec<String>, Vec<u8>) {
        let mut left = 0;
        let mut downloaded = 0;
        for p in &content.pieces {
            let piece = p.lock().unwrap();
            match piece.status {
                PieceStatus::Available => downloaded += piece.size,
                _ => left += piece.size,
            }
        }
        println!("Connecting to tracker {:?}", tracker);
        let url = format!("{}{}info_hash={}&port=50658&uploaded=0&downloaded={}&left={}&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1",
            tracker,
            if tracker.contains('?') {"&"} else {"?"},
            info_hash,
            downloaded,
            left
        );
        println!("{:?}", url);

        http::get(&url)
    };
    let to_tracker_response = |body: Vec<u8>| -> Result<TrackerResponse, DecodeErrorWrapper> {
        let tr = TrackerResponse::from_bencode(&body);
        match tr {
            Ok(tr) => Ok(tr),
            Err(e) => Err(DecodeErrorWrapper { derr: e }),
        }
    };

    let mut result;

    if tf.announce_list != None {
        for tracker_list in tf.announce_list.as_ref().unwrap() {
            for tracker in tracker_list {
                result = conn(
                    tracker,
                    &tf.info_hash.as_string_url_encoded(),
                );
                match to_tracker_response(result.2) {
                    Ok(r) => return Some(r),
                    Err(e) => println!("{}", e.unwrap()),
                }
            }
        }
    } else {
        result = conn(
            &tf.announce,
            &tf.info_hash.as_string_url_encoded(),
        );
        match to_tracker_response(result.2) {
            Ok(r) => return Some(r),
            Err(e) => println!("{}", e.unwrap()),
        }
    }
    None
}

/*
    Why the hell bendy removed
        impl<T: StdError + Send + Sync + 'static> From<T> for Error
    I don't know, but now I have to write this ugly piece of code
*/
#[derive(Debug)]
struct DecodeErrorWrapper {
    derr: DecodeError,
}

impl<T: std::error::Error + Send + Sync + 'static> From<T> for DecodeErrorWrapper {
    fn from(error: T) -> Self {
        let derr = DecodeError::malformed_content(error);
        DecodeErrorWrapper { derr }
    }
}

impl DecodeErrorWrapper {
    fn unwrap(self) -> DecodeError {
        self.derr
    }
}
