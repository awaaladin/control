//! A deliberately tiny HTTP/1.0 client. Router admin pages and UPnP
//! description files are plain HTTP on the LAN; pulling in a full client
//! (and a TLS stack) would bloat the binary we later want on the router.

use anyhow::{anyhow, bail, Result};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn title(&self) -> Option<String> {
        let lower = self.body.to_ascii_lowercase();
        let open = lower.find("<title")?;
        let start = open + lower[open..].find('>')? + 1;
        let end = start + lower[start..].find("</title")?;
        let t = collapse_ws(&self.body[start..end]);
        (!t.is_empty()).then_some(t)
    }

    /// One-line summary: status, server software, page title, redirect.
    pub fn summary(&self) -> String {
        let mut s = format!("HTTP {}", self.status);
        if let Some(v) = self.header("server") {
            s += &format!(" · server: {v}");
        }
        if let Some(t) = self.title() {
            s += &format!(" · title: \"{}\"", truncate(&t, 60));
        }
        if let Some(l) = self.header("location") {
            s += &format!(" · → {}", truncate(l, 60));
        }
        if let Some(a) = self.header("www-authenticate") {
            s += &format!(" · auth: {}", truncate(a, 40));
        }
        s
    }
}

/// A request for [`request`]. `body` is sent as form data.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a str>,
}

pub async fn get(host: &str, port: u16, path: &str, timeout: Duration) -> Result<HttpResponse> {
    let req = Request { method: "GET", path, headers: &[], body: None };
    request(host, port, &req, timeout).await
}

pub async fn request(host: &str, port: u16, req: &Request<'_>, timeout: Duration) -> Result<HttpResponse> {
    tokio::time::timeout(timeout, request_inner(host, port, req))
        .await
        .map_err(|_| anyhow!("timed out"))?
}

async fn request_inner(host: &str, port: u16, req: &Request<'_>) -> Result<HttpResponse> {
    let mut stream = TcpStream::connect((host, port)).await?;
    let host_hdr = if port == 80 { host.to_string() } else { format!("{host}:{port}") };
    let mut head = format!(
        "{} {} HTTP/1.0\r\nHost: {host_hdr}\r\nUser-Agent: rhcs/0.1\r\nAccept: */*\r\nConnection: close\r\n",
        req.method, req.path
    );
    for (k, v) in req.headers {
        head += &format!("{k}: {v}\r\n");
    }
    if let Some(body) = req.body {
        head += &format!(
            "Content-Type: application/x-www-form-urlencoded; charset=UTF-8\r\nContent-Length: {}\r\n",
            body.len()
        );
    }
    head += "\r\n";
    if let Some(body) = req.body {
        head += body;
    }
    stream.write_all(head.as_bytes()).await?;

    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > MAX_BYTES {
                    break;
                }
            }
            // Embedded servers often reset instead of closing cleanly.
            Err(_) if !raw.is_empty() => break,
            Err(e) => return Err(e.into()),
        }
    }
    parse(&raw)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Where the headers end. Embedded servers mix `\n` and `\r\n` line endings
/// (the ZTE MF293N ends some header lines with a bare `\n`, so its blank line
/// shows up as `\n\r\n`), so take the earliest blank line in any style.
fn header_end(raw: &[u8]) -> (usize, usize) {
    [&b"\r\n\r\n"[..], b"\n\r\n", b"\n\n"]
        .iter()
        .filter_map(|sep| find(raw, sep).map(|i| (i, sep.len())))
        .min_by_key(|(i, _)| *i)
        .unwrap_or((raw.len(), 0))
}

pub fn parse(raw: &[u8]) -> Result<HttpResponse> {
    let (head_end, sep) = header_end(raw);
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let mut lines = head.lines();
    let status_line = lines.next().unwrap_or_default();
    if !status_line.starts_with("HTTP/") {
        bail!("not an HTTP response");
    }
    let status = status_line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut body = raw[(head_end + sep).min(raw.len())..].to_vec();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        body = dechunk(&body);
    }
    Ok(HttpResponse { status, headers, body: String::from_utf8_lossy(&body).into_owned() })
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(i) = find(b, b"\r\n") {
        let size_line = String::from_utf8_lossy(&b[..i]);
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = i + 2;
        let end = (start + size).min(b.len());
        out.extend_from_slice(&b[start..end]);
        if end + 2 > b.len() {
            break;
        }
        b = &b[end + 2..];
    }
    out
}

/// Splits `http://host:port/path` into parts. HTTPS is not supported.
pub fn parse_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.trim().strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (authority, 80),
    };
    Some((host.to_string(), port, path))
}

pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_and_title() {
        let raw = b"HTTP/1.1 200 OK\r\nServer: uhttpd\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n<titl\r\n13\r\ne>LuCI Admin</title>\r\n0\r\n\r\n";
        let r = parse(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.header("SERVER"), Some("uhttpd"));
        assert_eq!(r.title().as_deref(), Some("LuCI Admin"));
    }

    #[test]
    fn mixed_line_endings_from_zte() {
        // Byte-for-byte layout of the MF293N's reply to the LOGIN POST (2026-11-12):
        // status line and Set-Cookie end in a bare \n, the rest in \r\n.
        let raw = b"HTTP/1.0 200 OK\nPragma: no-cache\r\nCache-control: no-cache\r\nContent-Type: text/html\r\n\
X-Frame-Options: sameorigin\r\nX-XSS-Protection: 1; mode=block\r\n\
Set-Cookie: zsidn=SESSION;path=/;HttpOnly=1;Expires=Mon, 1 Jan 2050 00:00:00 GMT\n\r\n{\"result\":\"0\"}";
        let r = parse(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "{\"result\":\"0\"}");
        assert!(r.header("set-cookie").unwrap().starts_with("zsidn=SESSION"));
    }

    #[test]
    fn url_parts() {
        assert_eq!(
            parse_url("http://192.168.1.1:49152/rootDesc.xml"),
            Some(("192.168.1.1".into(), 49152, "/rootDesc.xml".into()))
        );
        assert_eq!(parse_url("http://10.0.0.1"), Some(("10.0.0.1".into(), 80, "/".into())));
        assert_eq!(parse_url("https://x"), None);
    }
}
