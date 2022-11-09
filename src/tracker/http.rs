use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};

/// Send GET with fixed headers
pub fn get(url: &str) -> (u16, Vec<String>, Vec<u8>) {
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
    stream
        .write_all(body.as_bytes())
        .expect("Cannot write bytes");

    let mut response = vec![];
    stream
        .read_to_end(&mut response)
        .expect("Cannot read response");

    let response = parse_lines(get_lines(&response));
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
    //TODO: PARSE STATUS CODE!!!!
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
