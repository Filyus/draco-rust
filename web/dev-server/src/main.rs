use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use local_ip_address::list_afinet_netifas;
use miniz_oxide::deflate::compress_to_vec;

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("www"));
    let port = args.next().unwrap_or_else(|| "8080".to_string());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
    listener.set_nonblocking(true)?;
    let running = Arc::new(AtomicBool::new(true));
    let shutdown = running.clone();

    ctrlc::set_handler(move || {
        shutdown.store(false, Ordering::SeqCst);
    })
    .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

    println!("Serving from: {}", root.display());
    println!("WASM gzip compression: enabled");
    println!("Listening on all IPv4 interfaces");
    println!("URLs:");
    for url in server_urls(&port) {
        println!("  {url}");
    }
    println!("Press Ctrl+C to stop the server");

    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                spawn_connection_handler(stream, root.clone());
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => eprintln!("Connection failed: {err}"),
        }
    }

    println!("Server stopped");
    Ok(())
}

fn spawn_connection_handler(stream: TcpStream, root: PathBuf) {
    thread::spawn(move || {
        if let Err(err) = stream.set_nonblocking(false) {
            eprintln!("Request failed: {err}");
            return;
        }

        if let Err(err) = handle_connection(stream, &root) {
            if err.kind() == io::ErrorKind::WouldBlock {
                return;
            }

            eprintln!("Request failed: {err}");
        }
    });
}

fn server_urls(port: &str) -> Vec<String> {
    let mut addresses = vec![Ipv4Addr::LOCALHOST];

    if let Ok(interfaces) = list_afinet_netifas() {
        for (_, address) in interfaces {
            let std::net::IpAddr::V4(address) = address else {
                continue;
            };

            if !addresses.contains(&address) {
                addresses.push(address);
            }
        }
    }

    addresses
        .into_iter()
        .map(|address| format!("http://{address}:{port}"))
        .collect()
}

fn handle_connection(mut stream: TcpStream, root: &Path) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");

    let mut accepts_gzip = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }

        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("accept-encoding") && value.contains("gzip") {
                accepts_gzip = true;
            }
        }
    }

    if method != "GET" && method != "HEAD" {
        return write_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "text/plain",
            b"Method not allowed",
            false,
        );
    }

    let Some(path) = request_path(root, target) else {
        return write_response(
            &mut stream,
            400,
            "Bad Request",
            "text/plain",
            b"Bad request",
            false,
        );
    };

    let path = if path.is_dir() {
        path.join("index.html")
    } else {
        path
    };
    let Ok(body) = fs::read(&path) else {
        return write_response(
            &mut stream,
            404,
            "Not Found",
            "text/plain",
            b"Not found",
            false,
        );
    };

    let content_type = content_type(&path);
    if path.extension().is_some_and(|ext| ext == "wasm") && accepts_gzip {
        let compressed = gzip(&body);
        write_response_with_headers(
            &mut stream,
            200,
            "OK",
            content_type,
            &compressed,
            method == "HEAD",
            &[("Content-Encoding", "gzip"), ("Vary", "Accept-Encoding")],
        )
    } else {
        write_response(
            &mut stream,
            200,
            "OK",
            content_type,
            &body,
            method == "HEAD",
        )
    }
}

fn request_path(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    let decoded = percent_decode(path)?;
    let mut clean = PathBuf::new();

    for component in Path::new(decoded.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }

    Some(root.join(clean))
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = hex_value(*bytes.get(index + 1)?)?;
            let lo = hex_value(*bytes.get(index + 2)?)?;
            output.push((hi << 4) | lo);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(output).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(body.len());
    output.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255]);
    output.extend_from_slice(&compress_to_vec(body, 9));
    output.extend_from_slice(&crc32(body).to_le_bytes());
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff;

    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }

    !crc
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    headers_only: bool,
) -> io::Result<()> {
    write_response_with_headers(
        stream,
        status,
        reason,
        content_type,
        body,
        headers_only,
        &[],
    )
}

fn write_response_with_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    headers_only: bool,
    extra_headers: &[(&str, &str)],
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )?;

    for (name, value) in extra_headers {
        write!(stream, "{name}: {value}\r\n")?;
    }

    stream.write_all(b"\r\n")?;
    if !headers_only {
        stream.write_all(body)?;
    }

    Ok(())
}

fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();

    if let Some(content_type) = custom_content_type(extension) {
        return content_type;
    }

    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
}

fn custom_content_type(extension: &str) -> Option<&'static str> {
    match extension {
        "fbx" => Some("application/octet-stream"),
        "ply" => Some("application/octet-stream"),
        _ => None,
    }
}
