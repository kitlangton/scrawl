//! Bounded resources for the local ingest HTTP transport. No external dependencies.

use std::io::{self, Read};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub const MAX_CONNECTIONS: usize = 8;
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
pub const BODY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Connections(Arc<AtomicUsize>);

pub struct Permit(Arc<AtomicUsize>);

impl Connections {
    pub fn acquire(&self) -> Option<Permit> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CONNECTIONS).then_some(active + 1)
            })
            .ok()
            .map(|_| Permit(Arc::clone(&self.0)))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn timeout_until(stream: &TcpStream, deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "request deadline exceeded",
        ));
    }
    stream.set_read_timeout(Some(remaining))
}

pub fn read_header(stream: &mut TcpStream, timeout: Duration) -> io::Result<String> {
    let deadline = Instant::now() + timeout;
    let mut header = Vec::with_capacity(2048);
    let mut byte = [0];
    while header.len() < 16_384 {
        timeout_until(stream, deadline)?;
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            return String::from_utf8(header)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP header"));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "HTTP header too large",
    ))
}

pub fn read_body(stream: &mut TcpStream, body: &mut [u8]) -> io::Result<()> {
    read_body_with_timeout(stream, body, BODY_TIMEOUT)
}

fn read_body_with_timeout(
    stream: &mut TcpStream,
    mut body: &mut [u8],
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    while !body.is_empty() {
        timeout_until(stream, deadline)?;
        match stream.read(body) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => body = &mut body[n..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{Shutdown, TcpListener};

    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (listener.accept().unwrap().0, client)
    }

    #[test]
    fn connection_limit_releases_capacity_on_drop() {
        let connections = Connections::default();
        let mut permits: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| connections.acquire().unwrap())
            .collect();
        assert!(connections.acquire().is_none());
        permits.pop();
        assert!(connections.acquire().is_some());
    }

    #[test]
    fn header_reader_preserves_the_body_and_rejects_incomplete_headers() {
        let (mut server, mut client) = pair();
        client
            .write_all(b"POST /ingest HTTP/1.1\r\nContent-Length: 4\r\n\r\ndata")
            .unwrap();
        assert!(
            read_header(&mut server, HEADER_TIMEOUT)
                .unwrap()
                .ends_with("\r\n\r\n")
        );
        let mut body = [0; 4];
        read_body(&mut server, &mut body).unwrap();
        assert_eq!(&body, b"data");

        let (mut server, mut client) = pair();
        client.write_all(b"GET / HTTP/1.1\r\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            read_header(&mut server, HEADER_TIMEOUT).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn oversized_headers_are_rejected() {
        let (mut server, mut client) = pair();
        client.write_all(&vec![b'x'; 16_384]).unwrap();
        assert_eq!(
            read_header(&mut server, HEADER_TIMEOUT).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn trickled_headers_have_a_total_deadline() {
        let (mut server, mut client) = pair();
        let writer = std::thread::spawn(move || {
            for _ in 0..20 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let started = Instant::now();
        let error = read_header(&mut server, Duration::from_millis(60)).unwrap_err();
        assert!(matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock));
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(server);
        writer.join().unwrap();
    }

    #[test]
    fn trickled_bodies_have_a_total_deadline() {
        let (mut server, mut client) = pair();
        let writer = std::thread::spawn(move || {
            for _ in 0..20 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let mut body = [0; 20];
        assert!(read_body_with_timeout(&mut server, &mut body, Duration::from_millis(60)).is_err());
        drop(server);
        writer.join().unwrap();
    }
}
