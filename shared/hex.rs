//! Hex (the local dictation app) as an optional transcriber, through its desktop local API v2
//! (`HEX_APPLICATION_SUPPORT_DIR` or `~/Library/Application Support/hex/local-api.json`, or
//! `voice-control` before Hex moved it, holds the port and bearer token; `GET /health`, `GET /models`, `POST /transcriptions`).
//!
//! Hex returns text and a duration but no word times, and Apparent needs them to hand each
//! keyframe its words. So the recording is cut into speech islands (`audio::speech_islands`),
//! each island is transcribed on its own, and its words are spread across it by length. Those
//! times are estimates (`TranscriptData::estimated`): they place words inside the right island
//! but not on the right syllable, so spoken cues never cut keyframes from them.
//!
//! "Hex isn't connected" covers a missing discovery file, a refused connection, a rejected
//! token and any API version but 2. Nothing here logs or prints the token.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::capture::engine::runtime::audio::audio::{self, RecordedAudio};
use crate::shared::model::{TranscriptData, TranscriptSegment, WordTiming};

/// Tried in this order when no model is configured. Anything else is the user's explicit pick.
pub const PREFERRED_MODELS: [&str; 4] = [
    "parakeet_unified_en",
    "parakeet_v3",
    "parakeet_v2",
    "whisper_large_v3_turbo",
];

/// Keep the previous English default; Hex, not Apparent, validates model/language support.
pub const DEFAULT_LANGUAGE: &str = "en";

/// Accept language-code syntax without maintaining another list of Hex's languages.
/// Blank resets to English; disallow query delimiters before constructing the API URL.
pub fn language_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return Some(DEFAULT_LANGUAGE.into());
    }
    (value.len() <= 35
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && value.as_bytes()[0].is_ascii_alphabetic())
    .then(|| value.to_ascii_lowercase())
}
const SAMPLE_RATE_KHZ: u64 = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const QUICK_READ_TIMEOUT: Duration = Duration::from_secs(2);
/// A cold model may need to load before the first answer.
const TRANSCRIBE_READ_TIMEOUT: Duration = Duration::from_secs(90);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const TOTAL_DEADLINE: Duration = Duration::from_secs(180);
const RESPONSE_LIMIT: u64 = 8 * 1024 * 1024;
const BUSY_RETRIES: u32 = 5;
const BUSY_BACKOFF: Duration = Duration::from_millis(200);

const MIN_ISLAND_MS: u64 = 120;
const MERGE_GAP_MS: u64 = 700;
const MAX_MERGED_MS: u64 = 30_000;
const MAX_FORCED_MERGE_MS: u64 = 8 * 60_000;
const MAX_CLIPS: usize = 24;
/// Extra audio on each side of an island sent to Hex, so edge words aren't clipped.
const CLIP_PAD_MS: u64 = 200;

#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    port: u16,
    token: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Endpoint {{ port: {}, token: <redacted> }}", self.port)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Discovery {
    port: u16,
    token: String,
    api_version: String,
}

/// Hex's support folder, as Hex's `app_paths::support_dir` resolves it: `~/Library/Application
/// Support/hex` on macOS (`voice-control` before Hex moved it), `%APPDATA%\voice-control` on Windows.
pub fn support_dir() -> PathBuf {
    std::env::var_os("HEX_APPLICATION_SUPPORT_DIR")
        .map(PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
            let legacy = data.join("voice-control");
            // Hex on macOS moved its data from voice-control to hex, leaving a symlink; an
            // unmigrated voice-control folder stays in use. Windows keeps voice-control.
            if cfg!(target_os = "macos")
                && (std::fs::read_link(&legacy).is_ok_and(|target| target.ends_with("hex"))
                    || !legacy.exists())
            {
                data.join("hex")
            } else {
                legacy
            }
        })
}

pub fn discovery_path() -> PathBuf {
    support_dir().join("local-api.json")
}

pub fn parse_discovery(bytes: &[u8]) -> Result<Endpoint> {
    // serde's messages can quote the offending value, so none of them are passed on.
    let Ok(discovery) = serde_json::from_slice::<Discovery>(bytes) else {
        bail!("Hex discovery file is not valid");
    };
    ensure!(
        discovery.api_version == "2",
        "Hex API version {:?} is not supported (need 2)",
        discovery.api_version
    );
    ensure!(discovery.port != 0, "Hex discovery file has no port");
    ensure!(
        !discovery.token.is_empty() && !discovery.token.contains(['\r', '\n']),
        "Hex discovery file has no usable token"
    );
    Ok(Endpoint {
        port: discovery.port,
        token: discovery.token,
    })
}

#[derive(Debug)]
struct Response {
    status: u16,
    body: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Timeouts {
    read: Duration,
}

fn request(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: &[u8],
    timeouts: Timeouts,
) -> Result<Response> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let mut stream =
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).context("Could not reach Hex")?;
    stream.set_write_timeout(Some(UPLOAD_TIMEOUT))?;
    stream.set_read_timeout(Some(timeouts.read))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.port,
        endpoint.token,
        body.len()
    )?;
    // Hex can reject before it has read the whole upload; keep its answer over a broken pipe.
    let upload = stream.write_all(body);
    let mut bytes = Vec::new();
    let read = stream.take(RESPONSE_LIMIT + 1).read_to_end(&mut bytes);
    if bytes.is_empty() {
        read?;
    }
    ensure!(
        bytes.len() as u64 <= RESPONSE_LIMIT,
        "Hex response too large"
    );
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .context("Invalid Hex HTTP response")?;
    let header = std::str::from_utf8(&bytes[..split]).context("Invalid Hex HTTP header")?;
    let status: u16 = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("Missing Hex HTTP status")?
        .parse()
        .context("Invalid Hex HTTP status")?;
    let body = bytes[split + 4..].to_vec();
    if let Some(length) = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
    {
        ensure!(body.len() == length, "Incomplete Hex HTTP response");
    }
    if status == 200 {
        upload.context("Hex upload failed")?;
    }
    Ok(Response { status, body })
}

fn error_code(body: &[u8]) -> String {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    value
        .get("code")
        .or_else(|| value.get("error").and_then(|error| error.get("code")))
        .and_then(|code| code.as_str())
        .unwrap_or("unknown")
        .to_owned()
}

/// Why a transcription request failed.
#[derive(Debug)]
pub enum Failure {
    /// Hex's queue is full (`429 queue-full`): try again shortly.
    Busy,
    /// Hex refused this audio or model (`4xx`), which retrying won't change.
    Rejected { status: u16, code: String },
    /// Anything else: Hex is gone, errored or too slow.
    Unavailable(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "Hex is busy"),
            Self::Rejected { status, code } => {
                write!(f, "Hex rejected the audio ({status} {code})")
            }
            Self::Unavailable(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for Failure {}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub verified: bool,
}

impl Model {
    /// Ready to transcribe now. Apparent never asks Hex to download or prepare a model.
    pub fn usable(&self) -> bool {
        self.installed && self.verified
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub version: String,
    pub models: Vec<Model>,
}

impl Status {
    pub fn usable_models(&self) -> impl Iterator<Item = &Model> {
        self.models.iter().filter(|model| model.usable())
    }
}

/// The configured model when Hex has it ready, else the first ready one of `PREFERRED_MODELS`.
pub fn pick_model(models: &[Model], configured: Option<&str>) -> Option<String> {
    let usable = |id: &str| models.iter().any(|model| model.id == id && model.usable());
    configured
        .filter(|id| usable(id))
        .or_else(|| PREFERRED_MODELS.into_iter().find(|id| usable(id)))
        .map(str::to_owned)
}

#[derive(Debug)]
pub struct Client {
    endpoint: Endpoint,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    version: String,
    api_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transcription {
    transcript: String,
}

impl Client {
    /// Connects to the running Hex, or says why it can't.
    pub fn connect() -> Result<Self> {
        Self::connect_at(&discovery_path())
    }

    pub fn connect_at(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).context("Hex is not running (no discovery file)")?;
        let client = Self {
            endpoint: parse_discovery(&bytes)?,
        };
        client.health()?;
        Ok(client)
    }

    fn get(&self, path: &str) -> Result<Response> {
        let response = request(
            &self.endpoint,
            "GET",
            path,
            &[],
            Timeouts {
                read: QUICK_READ_TIMEOUT,
            },
        )?;
        ensure!(
            response.status != 401,
            "Hex rejected Apparent's credentials (restart Hex?)"
        );
        ensure!(
            response.status == 200,
            "Hex answered {path} with HTTP {}",
            response.status
        );
        Ok(response)
    }

    /// Hex's version, after checking it speaks API 2.
    pub fn health(&self) -> Result<String> {
        let health: Health = serde_json::from_slice(&self.get("/health")?.body)
            .context("Invalid Hex health response")?;
        ensure!(
            health.api_version == "2",
            "Hex API version {:?} is not supported (need 2)",
            health.api_version
        );
        Ok(health.version)
    }

    pub fn models(&self) -> Result<Vec<Model>> {
        serde_json::from_slice(&self.get("/models")?.body).context("Invalid Hex model list")
    }

    pub fn status(&self) -> Result<Status> {
        Ok(Status {
            version: self.health()?,
            models: self.models()?,
        })
    }

    /// One transcription request for a complete WAV.
    pub fn transcribe(&self, wav: &[u8], model: &str, language: &str) -> Result<String, Failure> {
        let language = language_code(language).ok_or_else(|| Failure::Rejected {
            status: 400,
            code: "invalid-language".into(),
        })?;
        let response = request(
            &self.endpoint,
            "POST",
            &format!("/transcriptions?model={model}&language={language}"),
            wav,
            Timeouts {
                read: TRANSCRIBE_READ_TIMEOUT,
            },
        )
        .map_err(|error| Failure::Unavailable(format!("{error:#}")))?;
        match response.status {
            200 => serde_json::from_slice::<Transcription>(&response.body)
                .map(|transcription| transcription.transcript)
                .map_err(|_| Failure::Unavailable("Invalid Hex transcription response".into())),
            429 => Err(Failure::Busy),
            status @ (400 | 408 | 411 | 413 | 415) => Err(Failure::Rejected {
                status,
                code: error_code(&response.body),
            }),
            status => Err(Failure::Unavailable(format!(
                "Hex transcription failed (HTTP {status} {})",
                error_code(&response.body)
            ))),
        }
    }

    fn transcribe_retrying(
        &self,
        wav: &[u8],
        model: &str,
        language: &str,
    ) -> Result<String, Failure> {
        let mut delay = BUSY_BACKOFF;
        for _ in 0..BUSY_RETRIES {
            match self.transcribe(wav, model, language) {
                Err(Failure::Busy) => {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
                other => return other,
            }
        }
        self.transcribe(wav, model, language)
    }
}

pub fn status() -> Result<Status> {
    Client::connect()?.status()
}

static CACHE: Mutex<Option<Option<Status>>> = Mutex::new(None);
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// The last known Hex status without touching the network, for the menu. `None` means Hex
/// wasn't connected at the last check (or none has finished yet).
pub fn cached_status() -> Option<Status> {
    CACHE.lock().ok()?.clone()?
}

/// Re-checks Hex on a background thread (one at a time).
pub fn refresh_in_background() {
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("apparent-hex-status".into())
        .spawn(|| {
            let status = status().ok();
            if let Ok(mut cache) = CACHE.lock() {
                *cache = Some(status);
            }
            REFRESHING.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        REFRESHING.store(false, Ordering::Release);
    }
}

/// Keeps `cached_status` fresh while the app runs. A missing discovery file makes each check a
/// single failed file read.
pub fn watch() {
    let _ = std::thread::Builder::new()
        .name("apparent-hex-watch".into())
        .spawn(|| {
            loop {
                let status = status().ok();
                if let Ok(mut cache) = CACHE.lock() {
                    *cache = Some(status);
                }
                std::thread::sleep(Duration::from_secs(10));
            }
        });
}

/// Speech spans worth one request each: tiny blips dropped, near neighbours joined so a clip
/// holds whole phrases, and never more than `MAX_CLIPS`.
pub fn plan_clips(islands: &[(u64, u64)], duration_ms: u64) -> Vec<(u64, u64)> {
    let mut clips: Vec<(u64, u64)> = Vec::new();
    for &(start, end) in islands {
        let (start, end) = (start.min(duration_ms), end.min(duration_ms));
        if end.saturating_sub(start) < MIN_ISLAND_MS {
            continue;
        }
        match clips.last_mut() {
            Some(last)
                if start.saturating_sub(last.1) < MERGE_GAP_MS && end - last.0 <= MAX_MERGED_MS =>
            {
                last.1 = end;
            }
            _ => clips.push((start, end)),
        }
    }
    while clips.len() > MAX_CLIPS {
        let Some(index) = (1..clips.len())
            .filter(|&i| clips[i].1 - clips[i - 1].0 <= MAX_FORCED_MERGE_MS)
            .min_by_key(|&i| clips[i].0 - clips[i - 1].1)
        else {
            break;
        };
        clips[index - 1].1 = clips[index].1;
        clips.remove(index);
    }
    clips
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', ';', ':'])
}

/// Spreads `text` over `start..end`, each word's share weighted by its length. Words tile the
/// span exactly (no gaps, no overlap), so the first begins at `start` and the last ends at `end`.
pub fn spread_words(text: &str, start: u64, end: u64) -> (Vec<TranscriptSegment>, Vec<WordTiming>) {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let end = end.max(start);
    let total: u128 = tokens
        .iter()
        .map(|token| token.chars().count().max(1) as u128)
        .sum();
    let mut words: Vec<WordTiming> = Vec::with_capacity(tokens.len());
    let mut segments = Vec::new();
    let mut segment_start = 0;
    let mut weight: u128 = 0;
    for (index, token) in tokens.iter().enumerate() {
        let at = |weight: u128| start + (u128::from(end - start) * weight / total) as u64;
        let t0 = at(weight);
        weight += token.chars().count().max(1) as u128;
        words.push(WordTiming {
            t0,
            t1: at(weight),
            text: (*token).to_owned(),
        });
        if ends_sentence(token) || index + 1 == tokens.len() {
            let phrase = &words[segment_start..];
            segments.push(TranscriptSegment {
                t0: phrase[0].t0,
                t1: phrase[phrase.len() - 1].t1,
                text: phrase
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            segment_start = words.len();
        }
    }
    (segments, words)
}

/// Transcribes each clip with `send` (given that clip's WAV) and spreads its words over it.
/// A clip Hex refuses is skipped; any other failure (or every clip refused) fails the whole
/// recording so the caller can use another transcriber.
pub fn transcribe_clips(
    samples: &[f32],
    clips: &[(u64, u64)],
    model: &str,
    mut send: impl FnMut(&[u8]) -> Result<String, Failure>,
) -> Result<TranscriptData> {
    let deadline = Instant::now() + TOTAL_DEADLINE;
    let mut transcript = TranscriptData {
        source: format!("hex:{model}"),
        model: Some(model.to_owned()),
        estimated: true,
        ..Default::default()
    };
    let mut rejected = 0;
    let mut texts: Vec<String> = Vec::new();
    for &(start, end) in clips {
        ensure!(Instant::now() < deadline, "Hex took too long");
        let from = (start.saturating_sub(CLIP_PAD_MS) * SAMPLE_RATE_KHZ) as usize;
        let to = (((end + CLIP_PAD_MS) * SAMPLE_RATE_KHZ) as usize).min(samples.len());
        if from >= to {
            continue;
        }
        let wav = audio::encode_wav_16k(&samples[from..to])?;
        let text = match send(&wav) {
            Ok(text) => text,
            Err(Failure::Rejected { status, code }) => {
                crate::capture::engine::runtime::apparent_log(format!(
                    "Hex rejected a clip ({status} {code}); skipping it"
                ));
                rejected += 1;
                continue;
            }
            Err(failure) => return Err(failure.into()),
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let (segments, words) = spread_words(&text, start, end);
        transcript.segments.extend(segments);
        transcript.words.extend(words);
        texts.push(text);
    }
    ensure!(
        rejected == 0 || rejected < clips.len(),
        "Hex refused every clip"
    );
    transcript.text = texts.join(" ");
    Ok(transcript)
}

/// Transcribes a recording with `model`, as `transcribe_clips` over its speech islands.
pub fn transcribe_audio(
    client: &Client,
    model: &str,
    language: &str,
    audio: &RecordedAudio,
) -> Result<TranscriptData> {
    let samples = audio::decode_wav_16k(&audio.wav_bytes)?;
    let duration_ms = samples.len() as u64 / SAMPLE_RATE_KHZ;
    let mut clips = plan_clips(&audio::speech_islands(&samples), duration_ms);
    if clips.is_empty() && duration_ms >= MIN_ISLAND_MS * 4 {
        // Too quiet to find speech by energy: let Hex hear the whole thing.
        clips.push((0, duration_ms));
    }
    transcribe_clips(&samples, &clips, model, |wav| {
        client.transcribe_retrying(wav, model, language)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    struct Fake {
        port: u16,
        requests: Arc<Mutex<Vec<(String, String, usize)>>>,
    }

    /// Serves `respond(method, path, authorized)` on a loopback port until the test ends.
    fn fake_hex(respond: impl Fn(&str, &str, bool) -> (u16, String) + Send + 'static) -> Fake {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let path = parts.next().unwrap_or_default().to_owned();
                let (mut length, mut authorized) = (0, false);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    let lower = header.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    authorized |= header.trim() == "Authorization: Bearer secret-token";
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), length));
                let (status, text) = respond(&method, &path, authorized);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        Fake { port, requests }
    }

    fn client(port: u16) -> Client {
        Client {
            endpoint: Endpoint {
                port,
                token: "secret-token".into(),
            },
        }
    }

    fn model(id: &str, installed: bool, verified: bool) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            installed,
            verified,
        }
    }

    #[test]
    fn discovery_needs_api_two_a_port_and_a_token() {
        let ok = parse_discovery(br#"{"port":62652,"token":"hunter2","apiVersion":"2","pid":9}"#)
            .unwrap();
        assert_eq!(ok.port, 62652);
        assert!(!format!("{ok:?}").contains("hunter2"));
        for bad in [
            &br#"{"port":1,"token":"t","apiVersion":"1"}"#[..],
            br#"{"port":0,"token":"t","apiVersion":"2"}"#,
            br#"{"port":1,"token":"","apiVersion":"2"}"#,
            br#"{"port":1,"token":"a\nb","apiVersion":"2"}"#,
            br#"{"port":"secret-token","token":"t","apiVersion":"2"}"#,
            br#"{"token":"t","apiVersion":"2"}"#,
            b"not json",
        ] {
            let error = format!("{:#}", parse_discovery(bad).unwrap_err());
            assert!(!error.contains("secret-token"), "{error}");
        }
    }

    #[test]
    fn missing_discovery_or_refused_connection_is_not_connected() {
        let dir = std::env::temp_dir().join(format!("apparent-hex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Client::connect_at(&dir.join("nope.json")).is_err());
        let closed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let path = dir.join("local-api.json");
        std::fs::write(
            &path,
            format!(r#"{{"port":{port},"token":"t","apiVersion":"2"}}"#),
        )
        .unwrap();
        assert!(Client::connect_at(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn connect_checks_token_and_api_version() {
        let dir = std::env::temp_dir().join(format!("apparent-hex-v-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = fake_hex(|_, path, authorized| {
            match (path, authorized) {
            (_, false) => (401, String::new()),
            ("/health", true) => (200, r#"{"version":"2.2.0","apiVersion":"2"}"#.into()),
            ("/models", true) => (
                200,
                r#"[{"id":"parakeet_unified_en","name":"P","installed":true,"verified":true,"extra":1}]"#
                    .into(),
            ),
            _ => (404, String::new()),
        }
        });
        let write = |token: &str| {
            let path = dir.join("local-api.json");
            std::fs::write(
                &path,
                format!(
                    r#"{{"port":{},"token":"{token}","apiVersion":"2"}}"#,
                    fake.port
                ),
            )
            .unwrap();
            path
        };
        let status = Client::connect_at(&write("secret-token"))
            .unwrap()
            .status()
            .unwrap();
        assert_eq!(status.version, "2.2.0");
        assert_eq!(status.usable_models().count(), 1);
        let error = Client::connect_at(&write("wrong")).unwrap_err();
        assert!(!format!("{error:#}").contains("wrong"));

        let old = fake_hex(|_, _, _| (200, r#"{"version":"1.9","apiVersion":"1"}"#.into()));
        assert!(client(old.port).health().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn models_must_be_installed_and_verified_and_default_order_wins() {
        let models = vec![
            model("parakeet_v3", true, true),
            model("parakeet_unified_en", true, false),
            model("parakeet_v2", false, false),
            model("qwen3_asr06_b", true, true),
        ];
        assert_eq!(pick_model(&models, None).as_deref(), Some("parakeet_v3"));
        assert_eq!(
            pick_model(&models, Some("qwen3_asr06_b")).as_deref(),
            Some("qwen3_asr06_b")
        );
        // A configured model that isn't ready falls back to the default order.
        assert_eq!(
            pick_model(&models, Some("parakeet_v2")).as_deref(),
            Some("parakeet_v3")
        );
        assert_eq!(
            pick_model(&[model("qwen3_asr06_b", true, true)], None),
            None
        );
        assert_eq!(pick_model(&[], Some("parakeet_v3")), None);
    }

    #[test]
    fn language_is_sent_to_hex_without_injecting_query_parameters() {
        let fake = fake_hex(|_, _, _| (200, r#"{"transcript":"tekst"}"#.into()));
        for language in ["nl", "auto", "en"] {
            client(fake.port)
                .transcribe(b"RIFF", "m", language)
                .unwrap();
        }
        let paths: Vec<_> = fake
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, path, _)| path.clone())
            .collect();
        assert_eq!(
            paths,
            [
                "/transcriptions?model=m&language=nl",
                "/transcriptions?model=m&language=auto",
                "/transcriptions?model=m&language=en",
            ]
        );
        assert!(
            client(fake.port)
                .transcribe(b"RIFF", "m", "nl&language=en")
                .is_err()
        );
        assert_eq!(fake.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn language_codes_allow_future_languages_but_not_url_syntax() {
        assert_eq!(language_code("").as_deref(), Some("en"));
        assert_eq!(language_code(" NL ").as_deref(), Some("nl"));
        assert_eq!(
            language_code("new-Language").as_deref(),
            Some("new-language")
        );
        for invalid in ["nl&x=y", "nl?", "en\r\nInjected", "*", "1", "two words"] {
            assert!(language_code(invalid).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn busy_is_retried_and_other_refusals_are_not() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let fake = fake_hex(move |_, _, _| {
            if seen.fetch_add(1, Ordering::SeqCst) < 2 {
                (429, r#"{"code":"queue-full"}"#.into())
            } else {
                (
                    200,
                    r#"{"transcript":"hello there","durationMs":900}"#.into(),
                )
            }
        });
        let text = client(fake.port)
            .transcribe_retrying(b"RIFF", "m", "nl")
            .unwrap();
        assert_eq!(text, "hello there");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let (method, path, length) = fake.requests.lock().unwrap()[0].clone();
        assert_eq!(method, "POST");
        assert_eq!(path, "/transcriptions?model=m&language=nl");
        assert_eq!(length, 4);
        assert!(fake.requests.lock().unwrap().iter()
            .all(|(_, path, _)| path == "/transcriptions?model=m&language=nl"));

        let refused = fake_hex(|_, _, _| (409, r#"{"code":"model-not-ready"}"#.into()));
        assert!(matches!(
            client(refused.port).transcribe(b"x", "m", "en"),
            Err(Failure::Unavailable(_))
        ));
        let bad = fake_hex(|_, _, _| (400, r#"{"code":"invalid-audio"}"#.into()));
        assert!(matches!(
            client(bad.port).transcribe(b"x", "m", "en"),
            Err(Failure::Rejected { status: 400, .. })
        ));
    }

    #[test]
    fn spreading_tiles_the_span_by_word_length() {
        let (segments, words) = spread_words("Hi there. Longer words follow", 1_000, 2_000);
        assert_eq!(words.first().unwrap().t0, 1_000);
        assert_eq!(words.last().unwrap().t1, 2_000);
        assert!(words.windows(2).all(|pair| pair[0].t1 == pair[1].t0));
        assert!(words.iter().all(|word| word.t0 <= word.t1));
        let length = |i: usize| words[i].t1 - words[i].t0;
        assert!(length(3) > length(0), "longer words get longer spans");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Hi there.");
        assert_eq!((segments[0].t0, segments[1].t1), (1_000, 2_000));
        assert_eq!(segments[0].t1, words[1].t1);
        assert_eq!(spread_words("  ", 0, 10), (vec![], vec![]));
        let (_, instant) = spread_words("a b", 500, 500);
        assert!(instant.iter().all(|word| word.t0 == 500 && word.t1 == 500));
    }

    #[test]
    fn clips_drop_blips_merge_neighbours_and_stay_bounded() {
        assert_eq!(
            plan_clips(
                &[(0, 50), (1_000, 1_500), (1_900, 2_400), (5_000, 5_300)],
                10_000
            ),
            vec![(1_000, 2_400), (5_000, 5_300)]
        );
        assert!(plan_clips(&[], 1_000).is_empty());
        let many: Vec<_> = (0..100).map(|i| (i * 2_000, i * 2_000 + 500)).collect();
        let clips = plan_clips(&many, 300_000);
        assert!(clips.len() <= MAX_CLIPS);
        assert_eq!(clips[0].0, 0);
        assert_eq!(clips.last().unwrap().1, 99 * 2_000 + 500);
        assert!(clips.windows(2).all(|pair| pair[0].1 <= pair[1].0));
        // A long unbroken stretch is not glued to its neighbour.
        assert_eq!(
            plan_clips(&[(0, 29_800), (30_000, 31_000)], 40_000).len(),
            2
        );
    }

    #[test]
    fn clip_transcripts_land_inside_their_clips_in_order() {
        let samples = vec![0.1_f32; 16_000 * 10];
        let mut texts = ["First part here.", "", "Then the second."].into_iter();
        let transcript = transcribe_clips(
            &samples,
            &[(500, 1_500), (3_000, 3_500), (6_000, 8_000)],
            "parakeet_v3",
            |_| Ok(texts.next().unwrap().to_owned()),
        )
        .unwrap();
        assert_eq!(transcript.source, "hex:parakeet_v3");
        assert_eq!(transcript.model.as_deref(), Some("parakeet_v3"));
        assert!(transcript.estimated);
        assert_eq!(transcript.text, "First part here. Then the second.");
        assert_eq!(transcript.words.len(), 6);
        assert!(
            transcript.words[..3]
                .iter()
                .all(|w| w.t0 >= 500 && w.t1 <= 1_500)
        );
        assert!(
            transcript.words[3..]
                .iter()
                .all(|w| w.t0 >= 6_000 && w.t1 <= 8_000)
        );
        assert!(
            transcript
                .words
                .windows(2)
                .all(|pair| pair[0].t1 <= pair[1].t0)
        );
        assert_eq!(transcript.segments.len(), 2);
    }

    #[test]
    fn a_refused_clip_is_skipped_but_other_failures_abort() {
        let samples = vec![0.1_f32; 16_000 * 4];
        let clips = [(0, 1_000), (2_000, 3_000)];
        let mut first = true;
        let transcript = transcribe_clips(&samples, &clips, "m", |_| {
            if std::mem::take(&mut first) {
                Err(Failure::Rejected {
                    status: 400,
                    code: "invalid-audio".into(),
                })
            } else {
                Ok("kept".into())
            }
        })
        .unwrap();
        assert_eq!(transcript.text, "kept");
        assert!(
            transcribe_clips(&samples, &clips, "m", |_| Err(Failure::Rejected {
                status: 400,
                code: "x".into()
            }))
            .is_err()
        );
        assert!(transcribe_clips(&samples, &clips, "m", |_| Err(Failure::Busy)).is_err());
        assert!(
            transcribe_clips(&samples, &clips, "m", |_| Err(Failure::Unavailable(
                "down".into()
            )))
            .is_err()
        );
    }

    #[test]
    fn transcribe_audio_sends_only_speech_to_the_fake_hex() {
        let fake = fake_hex(|_, path, _| {
            assert!(path.starts_with("/transcriptions?model=parakeet_v3"));
            (
                200,
                r#"{"transcript":"hello there friend","durationMs":1}"#.into(),
            )
        });
        let mut samples = vec![0.0_f32; 16_000 * 6];
        samples[16_000..16_000 * 2].fill(0.2);
        samples[16_000 * 4..16_000 * 5].fill(0.2);
        let audio = RecordedAudio {
            sample_count: samples.len(),
            duration_ms: 6_000,
            wav_bytes: audio::encode_wav_16k(&samples).unwrap(),
            saved_lease: None,
            live: Default::default(),
            #[cfg(target_os = "linux")]
            pause: Default::default(),
        };
        let transcript = transcribe_audio(&client(fake.port), "parakeet_v3", "en", &audio).unwrap();
        assert_eq!(fake.requests.lock().unwrap().len(), 2);
        assert_eq!(transcript.words.len(), 6);
        assert!(
            transcript.words[..3]
                .iter()
                .all(|w| w.t0 >= 1_000 && w.t1 <= 2_000)
        );
        assert!(
            transcript.words[3..]
                .iter()
                .all(|w| w.t0 >= 4_000 && w.t1 <= 5_000)
        );
    }
}
