//! Shared bounded JSONL framing for the versioned media channels.
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
type Result<T> = std::result::Result<T, &'static str>;

pub enum Input {
    Line(Vec<u8>),
    Closed,
    Invalid,
}

pub fn reader(max_line: usize) -> Receiver<Input> {
    let (tx, rx) = mpsc::sync_channel(32);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        loop {
            // take bounds allocation even for an unterminated hostile input line.
            let mut bytes = Vec::new();
            let n = (&mut stdin)
                .take(max_line as u64 + 2)
                .read_until(b'\n', &mut bytes);
            match n {
                Ok(0) => {
                    let _ = tx.send(Input::Closed);
                    break;
                }
                Ok(_) if bytes.ends_with(b"\n") => {
                    bytes.pop();
                    if bytes.ends_with(b"\r") {
                        bytes.pop();
                    }
                    if bytes.len() > max_line
                        || bytes.contains(&b'\r')
                        || std::str::from_utf8(&bytes).is_err()
                    {
                        let _ = tx.send(Input::Invalid);
                        break;
                    }
                    if tx.send(Input::Line(bytes)).is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = tx.send(Input::Invalid);
                    break;
                }
            }
        }
    });
    rx
}
pub fn send(value: Value, max_line: usize) -> Result<()> {
    let mut bytes = serde_json::to_vec(&value).map_err(|_| "internal")?;
    if bytes.len() > max_line {
        return Err("internal");
    }
    bytes.push(b'\n');
    let mut stdout = std::io::stdout().lock();
    let until = Instant::now() + Duration::from_secs(1);
    let mut sent = 0;
    while sent < bytes.len() {
        match stdout.write(&bytes[sent..]) {
            Ok(0) => return Err("internal"),
            Ok(n) => sent += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(_) => return Err("internal"),
        }
    }
    stdout.flush().map_err(|_| "internal")
}
