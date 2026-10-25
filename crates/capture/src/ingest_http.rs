//! Connection admission and header deadlines for ingest. Body reads keep their existing
//! per-read timeout, so a progressing upload can take longer than the header deadline.

use std::io::{self, Read};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub const MAX_CONNECTIONS: usize = 8;
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Connections(Arc<AtomicUsize>);

pub struct Permit(Arc<AtomicUsize>);

impl Connections {
    pub fn acquire(&self) -> Option<Permit> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
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

/// `None` is a clean connection close before a request. Always restore the socket's
/// original timeout before returning, including on failure; body readers do not inherit
/// the time left on the header deadline.
pub fn read_header(stream: &mut TcpStream, timeout: Duration) -> io::Result<Option<String>> {
    let previous_timeout = stream.read_timeout()?;
    let result = read_header_until(stream, Instant::now() + timeout);
    stream.set_read_timeout(previous_timeout)?;
    result
}

fn read_header_until(stream: &mut TcpStream, deadline: Instant) -> io::Result<Option<String>> {
    let mut header = Vec::with_capacity(2048);
    let mut byte = [0];
    while header.len() < 16_384 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP header deadline exceeded",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(&mut byte) {
            Ok(0) if header.is_empty() => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => header.push(byte[0]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
        if header.ends_with(b"\r\n\r\n") {
            return Ok(Some(String::from_utf8_lossy(&header).into_owned()));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "HTTP header too large",
    ))
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
    fn clean_close_before_a_request_is_not_an_error() {
        let (mut server, client) = pair();
        client.shutdown(Shutdown::Write).unwrap();
        assert!(read_header(&mut server, HEADER_TIMEOUT).unwrap().is_none());
    }

    #[test]
    fn header_reader_preserves_body_bytes_and_timeout() {
        let (mut server, mut client) = pair();
        let body_timeout = Some(Duration::from_secs(30));
        server.set_read_timeout(body_timeout).unwrap();
        client
            .write_all(b"POST /ingest HTTP/1.1\r\nContent-Length: 4\r\n\r\ndata")
            .unwrap();
        assert_eq!(
            read_header(&mut server, HEADER_TIMEOUT).unwrap().unwrap(),
            "POST /ingest HTTP/1.1\r\nContent-Length: 4\r\n\r\n"
        );
        assert_eq!(server.read_timeout().unwrap(), body_timeout);
        let mut body = [0; 4];
        server.read_exact(&mut body).unwrap();
        assert_eq!(&body, b"data");
    }

    #[test]
    fn truncated_headers_are_errors_and_restore_the_timeout() {
        let (mut server, mut client) = pair();
        let body_timeout = Some(Duration::from_secs(30));
        server.set_read_timeout(body_timeout).unwrap();
        client.write_all(b"GET / HTTP/1.1\r\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            read_header(&mut server, HEADER_TIMEOUT).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(server.read_timeout().unwrap(), body_timeout);
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
            for _ in 0..50 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let error = read_header(&mut server, Duration::from_millis(100)).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert_eq!(server.read_timeout().unwrap(), None);
        drop(server);
        writer.join().unwrap();
    }

    #[test]
    fn progressing_body_can_outlive_both_header_and_per_read_timeouts() {
        let (mut server, mut client) = pair();
        server
            .set_read_timeout(Some(Duration::from_millis(600)))
            .unwrap();
        client
            .write_all(b"POST /ingest HTTP/1.1\r\nContent-Length: 12\r\n\r\n")
            .unwrap();
        assert!(
            read_header(&mut server, Duration::from_millis(100))
                .unwrap()
                .is_some()
        );
        let started = Instant::now();
        let writer = std::thread::spawn(move || {
            for byte in b"abcdefghijkl" {
                std::thread::sleep(Duration::from_millis(60));
                client.write_all(&[*byte]).unwrap();
            }
        });
        let mut body = [0; 12];
        server.read_exact(&mut body).unwrap();
        assert_eq!(&body, b"abcdefghijkl");
        assert!(started.elapsed() > Duration::from_millis(600));
        writer.join().unwrap();
    }
}
