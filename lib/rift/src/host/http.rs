use percent_encoding::percent_decode_str;

pub struct Head {
    pub method: String,
    pub path: String,
    pub query: String,
    pub upgrade: bool,
    pub ws_key: Option<String>,
    pub len: usize,
}

// `Ok(None)` means more bytes are needed.
pub fn parse_head(buffer: &[u8]) -> Result<Option<Head>, ()> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    let len = match request.parse(buffer) {
        Ok(httparse::Status::Complete(len)) => len,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(_) => return Err(()),
    };
    let target = request.path.ok_or(())?;
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };
    let header = |name: &str| {
        request
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| String::from_utf8_lossy(header.value).into_owned())
    };
    let upgrade = header("upgrade").is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    Ok(Some(Head {
        method: request.method.ok_or(())?.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        upgrade,
        ws_key: header("sec-websocket-key"),
        len,
    }))
}

pub fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| {
            percent_decode_str(&value.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        })
    })
}

pub fn response(status: u16, reason: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

pub fn upgrade_response(accept_key: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept_key}\r\n\r\n"
    )
    .into_bytes()
}
