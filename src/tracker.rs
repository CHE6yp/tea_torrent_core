use crate::TorrentFile;
use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use std::io::Read;
use std::net::SocketAddr;
use ureq::{Error, Response};

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

pub fn connect_to_tracker(
    tf: &TorrentFile,
    allow_backup_trackers: bool,
) -> Option<TrackerResponse> {
    let conn = |tracker: &str, info_hash: &str, length: usize| -> Result<Response, ureq::Error> {
        println!("Connecting to tracker {:?}", tracker);
        let url = format!("{}{}info_hash={}&port=50658&uploaded=0&downloaded=0&left={}&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1",
            tracker,
            if tracker.contains('?') {"&"} else {"?"},
            info_hash,
            length
        );

        ureq::get(&url)
            .set("Content-Type", "application/octet-stream")
            .call()
    };
    let to_tracker_response = |resp: Response| -> Result<TrackerResponse, DecodeErrorWrapper> {
        let mut bytes: Vec<u8> = Vec::new();
        resp.into_reader().read_to_end(&mut bytes)?;
        let tr = TrackerResponse::from_bencode(&bytes);
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
                    tf.info.length,
                );
                match result {
                    Ok(r) => match to_tracker_response(r) {
                        Ok(r) => return Some(r),
                        Err(e) => println!("{}", e.unwrap()),
                    },
                    Err(Error::Status(code, response)) => {
                        println!(
                            "Error\nCode: {}\nResponse: {}",
                            code,
                            response.status_text()
                        )
                    }
                    Err(_) => {
                        println!("Some kind of io/transport error ");
                    }
                }
            }
        }
    } else {
        result = conn(
            &tf.announce,
            &tf.info_hash.as_string_url_encoded(),
            tf.info.length,
        );
        match result {
            Ok(r) => match to_tracker_response(r) {
                Ok(r) => return Some(r),
                Err(e) => println!("{}", e.unwrap()),
            },
            Err(Error::Status(code, response)) => {
                println!(
                    "Error\nCode: {}\nResponse: {}",
                    code,
                    response.status_text()
                )
            }
            Err(_) => { /* some kind of io/transport error */ }
        }
    }
    if allow_backup_trackers {
        for tracker in backup_trackers() {
            result = conn(
                &tracker,
                &tf.info_hash.as_string_url_encoded(),
                tf.info.length,
            );
            match result {
                Ok(r) => match to_tracker_response(r) {
                    Ok(r) => return Some(r),
                    Err(e) => println!("{}", e.unwrap()),
                },
                Err(Error::Status(code, response)) => {
                    println!(
                        "Error\nCode: {}\nResponse: {}",
                        code,
                        response.status_text()
                    )
                }
                Err(_) => {
                    println!("Some kind of io/transport error ");
                }
            }
        }
    }
    None
}

//BACKUP TRACKERS!!!
fn backup_trackers() -> [String; 25] {
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
