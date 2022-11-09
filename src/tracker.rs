use crate::TorrentFile;
use bendy::decoding::{Error as DecodeError, FromBencode, Object, ResultExt};
use std::io::Read;
use std::net::SocketAddr;

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

pub fn connect_to_tracker(tf: &TorrentFile) -> Option<TrackerResponse> {
    let conn = |tracker: &str, info_hash: &str, length: usize| -> (u16, Vec<String>, Vec<u8>) {
        println!("Connecting to tracker {:?}", tracker);
        let url = format!("{}{}info_hash={}&port=50658&uploaded=0&downloaded=0&left={}&corrupt=0&key=CFA4D362&event=started&numwant=200&compact=1&no_peer_id=1",
            tracker,
            if tracker.contains('?') {"&"} else {"?"},
            info_hash,
            length
        );

        stuff(&url)
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
                    tf.info.length,
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
            tf.info.length,
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

use std::io::Write;
use std::net::{Shutdown, TcpStream};

//host path
fn parse_url(url: &str) -> (String, u16, String) {
    let parsed = url::Url::parse(url).unwrap();
    let host = parsed.host_str().unwrap();
    let port = parsed.port_or_known_default();
    let path = parsed.path();
    let query = parsed.query().unwrap();

    (
        format!("{}", host),
        port.unwrap(),
        format!("{}?{}", path, query),
    )
}

fn stuff(url: &str) -> (u16, Vec<String>, Vec<u8>) {
    let announce_path = parse_url(url);
    let mut stream = TcpStream::connect(&format!("{}:{}", announce_path.0, announce_path.1))
        .expect("Cannot connect");

    let mut body = String::new();
    body.push_str(format!("GET {} HTTP/1.1", &announce_path.2).as_str());
    body.push_str("\r\n");
    body.push_str(format!("Host: {}", &announce_path.0).as_str());
    body.push_str("\r\n");
    body.push_str("User-Agent: teatorrent/0.0.3");
    body.push_str("\r\n");
    body.push_str("Accept: */*");
    body.push_str("\r\n");
    body.push_str("accept-encoding: gzip");
    body.push_str("\r\n");
    body.push_str("Content-Type: application/octet-stream");
    body.push_str("\r\n");
    body.push_str("Connection: close");
    body.push_str("\r\n");
    body.push_str("\r\n");
    println!("{}\r\n", body);
    stream
        .write_all(body.as_bytes())
        .expect("Cannot write bytes");

    println!("Newline {:?}", "\r\n".as_bytes());
    let mut response = vec![];
    let x = 13;
    stream
        .read_to_end(&mut response)
        .expect("Cannot read response");
    println!("{:?}", &response[0..x]);
    // println!("{:?}", response);
    // let response_slice = String::from_utf8_lossy(&response[0..x]);
    // let _response = String::from_utf8_lossy(&response);

    for line in get_lines(&response) {
        println!("{}", String::from_utf8_lossy(line));
    }

    let response = parse_lines(get_lines(&response));
    println!("{:?}", response);

    // println!("{}", response);
    // println!("{}", response_slice);
    stream.shutdown(Shutdown::Both).expect("Shutdown failed");
    response
}

fn get_lines(response: &Vec<u8>) -> Vec<&[u8]> {
    let mut start = 0;
    let mut lines = vec![];
    for index in 1..response.len() {
        if response[index - 1] == 13 && response[index] == 10 {
            lines.push(&response[start..index - 1]);
            start = index + 1;
        }
    }
    lines.push(&response[start..]);
    lines
}

fn parse_lines(lines: Vec<&[u8]>) -> (u16, Vec<String>, Vec<u8>) {
    let status_code: u16 = 0;
    let mut headers: Vec<String> = vec![];
    let body: Vec<u8>;

    let _status_code_string = String::from_utf8(lines[0].to_vec());
    for line in &lines[1..] {
        if line == &[] {
            break;
        }
        headers.push(String::from_utf8(line.to_vec()).unwrap());
    }

    body = lines.last().unwrap().to_vec();

    (status_code, headers, body)
}
