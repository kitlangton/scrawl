//! Which microphone captures record from: `"microphone": {"id", "name"}` in
//! `~/.config/apparent/config.json` (absent = the system default). The id is a CoreAudio UID
//! on macOS, a Pulse source name prefixed with `pulse:` on supported Linux desktops, or a CPAL
//! device id elsewhere; the name is kept to identify a missing device.

use serde::{Deserialize, Serialize};

/// The `microphone` key: a device the user picked in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub id: String,
    pub name: String,
}

/// The input devices connected now, in the host's order, and the default's id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub devices: Vec<Input>,
    pub default: Option<String>,
}

impl Inputs {
    pub fn find(&self, id: &str) -> Option<&Input> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn default_input(&self) -> Option<&Input> {
        self.default.as_deref().and_then(|id| self.find(id))
    }
}

/// What a recording uses, given the choice and what's connected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick<'a> {
    /// No choice: the system default (none when there's no microphone at all).
    Default(Option<&'a Input>),
    Chosen(&'a Input),
    /// The chosen device isn't connected; the default stands in.
    Missing {
        wanted: &'a Choice,
        using: Option<&'a Input>,
    },
}

impl<'a> Pick<'a> {
    pub fn new(choice: Option<&'a Choice>, inputs: &'a Inputs) -> Self {
        match choice {
            None => Pick::Default(inputs.default_input()),
            Some(wanted) => match inputs.find(&wanted.id) {
                Some(input) => Pick::Chosen(input),
                None => Pick::Missing {
                    wanted,
                    using: inputs.default_input(),
                },
            },
        }
    }

    /// The device that will record.
    pub fn device(&self) -> Option<&'a Input> {
        match *self {
            Pick::Default(input) | Pick::Missing { using: input, .. } => input,
            Pick::Chosen(input) => Some(input),
        }
    }

    /// "AirPods Pro isn't connected. Using MacBook Pro Microphone." for a missing device.
    pub fn note(&self) -> Option<String> {
        let Pick::Missing { wanted, using } = self else {
            return None;
        };
        Some(match using {
            Some(using) => format!("{} isn't connected. Using {}.", wanted.name, using.name),
            None => format!(
                "{} isn't connected, and there's no other microphone.",
                wanted.name
            ),
        })
    }
}

/// The configured choice, if any.
pub fn choice() -> Option<Choice> {
    choice_in(&crate::shared::handoff::default_config_path())
}

pub fn choice_in(path: &std::path::Path) -> Option<Choice> {
    crate::shared::handoff::load_config_from(path)
        .microphone
        .filter(|c| !c.id.trim().is_empty())
}

/// Saves a pick from Settings: `"default"` clears the key; an id that isn't connected (so it
/// can't be named) changes nothing.
pub fn save_in(path: &std::path::Path, id: &str, inputs: &Inputs) -> anyhow::Result<()> {
    let value = if id == "default" {
        serde_json::Value::Null
    } else if let Some(input) = inputs.find(id) {
        serde_json::to_value(Choice {
            id: input.id.clone(),
            name: input.name.clone(),
        })?
    } else {
        return Ok(());
    };
    crate::shared::handoff::set_config_value_in(path, "microphone", value)
}

/// Enumerates input devices (never on the UI thread).
pub fn inputs() -> Inputs {
    #[cfg(target_os = "linux")]
    if let Some(inputs) = crate::capture::desktop::linux::microphone::inputs() {
        return inputs;
    }
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map(|devices| {
            devices
                .filter_map(|device| {
                    let id = device.id().ok()?.id().to_owned();
                    let name = device.description().ok()?.name().to_owned();
                    Some(Input { id, name })
                })
                .collect()
        })
        .unwrap_or_default();
    let default = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.id().to_owned());
    Inputs { devices, default }
}

/// The cpal device for `id`, if connected.
pub fn device(id: &str) -> Option<cpal::Device> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host()
        .input_devices()
        .ok()?
        .find(|device| device.id().is_ok_and(|d| d.id() == id))
}

/// Settings-only input meter. It opens the effective device and never stores audio.
#[cfg(target_os = "macos")]
pub struct Meter;

#[cfg(target_os = "macos")]
impl Meter {
    pub fn start(_id: String) -> Self {
        Self
    }
    pub fn level(&self) -> f32 {
        0.0
    }
}

#[cfg(not(target_os = "macos"))]
pub struct Meter {
    stop: std::sync::mpsc::Sender<()>,
    level: std::sync::Arc<std::sync::atomic::AtomicU32>,
    arrival: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
}

#[cfg(not(target_os = "macos"))]
impl Meter {
    pub fn start(id: String) -> Self {
        let (stop, receiver) = std::sync::mpsc::channel();
        let level = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let arrival = std::sync::Arc::new(std::sync::Mutex::new(None));
        let output = level.clone();
        let observed = arrival.clone();
        std::thread::spawn(move || {
            if let Err(error) = Self::listen(&id, &receiver, output, observed) {
                crate::capture::engine::runtime::apparent_log(format!("Microphone meter: {error:#}"));
            }
        });
        Self {
            stop,
            level,
            arrival,
        }
    }

    pub fn level(&self) -> f32 {
        if !self.arrival.lock().is_ok_and(|at| {
            at.is_some_and(|time| time.elapsed() < std::time::Duration::from_millis(300))
        }) {
            return 0.0;
        }
        f32::from_bits(self.level.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn listen(
        id: &str,
        stop: &std::sync::mpsc::Receiver<()>,
        level: std::sync::Arc<std::sync::atomic::AtomicU32>,
        arrival: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    ) -> anyhow::Result<()> {
        use cpal::traits::{DeviceTrait, StreamTrait};
        #[cfg(target_os = "linux")]
        if let Some(source) = crate::capture::desktop::linux::microphone::source(id) {
            let _stream = crate::capture::desktop::linux::microphone::Stream::start(source, move |data| {
                let peak = data
                    .iter()
                    .map(|sample| sample.abs())
                    .fold(0.0_f32, f32::max)
                    .clamp(0.0, 1.0);
                level.store(peak.to_bits(), std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut at) = arrival.lock() {
                    *at = Some(std::time::Instant::now());
                }
            })?;
            let _ = stop.recv();
            return Ok(());
        }
        let device = device(id).ok_or_else(|| anyhow::anyhow!("selected input disappeared"))?;
        let supported = device.default_input_config()?;
        let config = supported.config();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => Self::stream::<f32>(&device, &config, level, arrival)?,
            cpal::SampleFormat::F64 => Self::stream::<f64>(&device, &config, level, arrival)?,
            cpal::SampleFormat::I8 => Self::stream::<i8>(&device, &config, level, arrival)?,
            cpal::SampleFormat::I16 => Self::stream::<i16>(&device, &config, level, arrival)?,
            cpal::SampleFormat::I24 => Self::stream::<cpal::I24>(&device, &config, level, arrival)?,
            cpal::SampleFormat::I32 => Self::stream::<i32>(&device, &config, level, arrival)?,
            cpal::SampleFormat::I64 => Self::stream::<i64>(&device, &config, level, arrival)?,
            cpal::SampleFormat::U8 => Self::stream::<u8>(&device, &config, level, arrival)?,
            cpal::SampleFormat::U16 => Self::stream::<u16>(&device, &config, level, arrival)?,
            cpal::SampleFormat::U24 => Self::stream::<cpal::U24>(&device, &config, level, arrival)?,
            cpal::SampleFormat::U32 => Self::stream::<u32>(&device, &config, level, arrival)?,
            cpal::SampleFormat::U64 => Self::stream::<u64>(&device, &config, level, arrival)?,
            format => anyhow::bail!("unsupported input meter sample format: {format}"),
        };
        stream.play()?;
        let _ = stop.recv();
        Ok(())
    }

    fn stream<T: cpal::SizedSample>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        level: std::sync::Arc<std::sync::atomic::AtomicU32>,
        arrival: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    ) -> anyhow::Result<cpal::Stream>
    where
        f32: cpal::FromSample<T>,
    {
        use cpal::traits::DeviceTrait;
        Ok(device.build_input_stream::<T, _, _>(
            config.clone(),
            move |data, _| {
                let peak = data
                    .iter()
                    .map(|sample| sample.to_sample::<f32>().abs())
                    .fold(0.0_f32, f32::max)
                    .clamp(0.0, 1.0);
                level.store(peak.to_bits(), std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut at) = arrival.lock() {
                    *at = Some(std::time::Instant::now());
                }
            },
            move |error| crate::capture::engine::runtime::apparent_log(format!("Microphone meter: {error}")),
            None,
        )?)
    }
}

#[cfg(not(target_os = "macos"))]
impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

/// Logs a missing chosen device once, until it's back or another goes missing.
pub fn note_pick(pick: &Pick) {
    static WARNED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let mut warned = WARNED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match (pick, pick.note()) {
        (Pick::Missing { wanted, .. }, Some(note)) => {
            if warned.as_deref() != Some(wanted.id.as_str()) {
                crate::capture::engine::runtime::apparent_log(format!("Microphone: {note}"));
                *warned = Some(wanted.id.clone());
            }
        }
        _ => *warned = None,
    }
}

/// Calls `changed` on its own thread once at the start, then whenever the device list or default
/// changes. CoreAudio has native notifications; other hosts check once a second while Settings
/// is open. Stops when dropped.
pub struct Watch {
    #[cfg(target_os = "macos")]
    sender: std::sync::mpsc::Sender<watch::Message>,
    #[cfg(not(target_os = "macos"))]
    stop: std::sync::mpsc::Sender<()>,
}

impl Watch {
    pub fn start(changed: impl Fn() + Send + 'static) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                sender: watch::start(Box::new(changed)),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (stop, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut previous = inputs();
                changed();
                while receiver
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .is_err_and(|e| e == std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    let current = inputs();
                    if current != previous {
                        previous = current;
                        changed();
                    }
                }
            });
            Self { stop }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        let _ = self.sender.send(watch::Message::Stop);
        #[cfg(not(target_os = "macos"))]
        let _ = self.stop.send(());
    }
}

#[cfg(target_os = "macos")]
mod watch {
    use std::ffi::c_void;
    use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
    use std::time::Duration;

    pub enum Message {
        Changed,
        Stop,
    }

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    type Listener = unsafe extern "C" fn(u32, u32, *const Address, *mut c_void) -> i32;

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectAddPropertyListener(
            object: u32,
            address: *const Address,
            listener: Listener,
            data: *mut c_void,
        ) -> i32;
        fn AudioObjectRemovePropertyListener(
            object: u32,
            address: *const Address,
            listener: Listener,
            data: *mut c_void,
        ) -> i32;
    }

    const SYSTEM_OBJECT: u32 = 1;
    const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    const DEVICES: u32 = u32::from_be_bytes(*b"dev#");
    const DEFAULT_INPUT: u32 = u32::from_be_bytes(*b"dIn ");

    unsafe extern "C" fn notify(_: u32, _: u32, _: *const Address, data: *mut c_void) -> i32 {
        // SAFETY: `data` is the boxed sender, freed only after both listeners are removed.
        let sender = unsafe { &*(data as *const Sender<Message>) };
        let _ = sender.send(Message::Changed);
        0
    }

    /// CoreAudio wants listeners added and removed on the same thread, so one thread owns them.
    pub fn start(changed: Box<dyn Fn() + Send>) -> Sender<Message> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let data = Box::into_raw(Box::new(sender.clone())) as *mut c_void as usize;
        let spawned = std::thread::Builder::new()
            .name("apparent-mic-watch".into())
            .spawn(move || {
                let data = data as *mut c_void;
                let addresses = [DEVICES, DEFAULT_INPUT].map(|selector| Address {
                    selector,
                    scope: SCOPE_GLOBAL,
                    element: 0,
                });
                for address in &addresses {
                    // SAFETY: a valid address and a listener that only reads `data`.
                    unsafe { AudioObjectAddPropertyListener(SYSTEM_OBJECT, address, notify, data) };
                }
                changed();
                run(&receiver, &*changed);
                for address in &addresses {
                    // SAFETY: the same triple that was added.
                    unsafe {
                        AudioObjectRemovePropertyListener(SYSTEM_OBJECT, address, notify, data)
                    };
                }
                // SAFETY: both listeners are gone, so nothing reads `data` any more.
                drop(unsafe { Box::from_raw(data as *mut Sender<Message>) });
            });
        if let Err(error) = spawned {
            crate::capture::engine::runtime::apparent_log(format!("Microphone watch failed: {error}"));
        }
        sender
    }

    fn run(receiver: &Receiver<Message>, changed: &dyn Fn()) {
        while let Ok(Message::Changed) = receiver.recv() {
            // Plugging one device in fires several notifications; settle before reading.
            loop {
                match receiver.recv_timeout(Duration::from_millis(150)) {
                    Ok(Message::Changed) => continue,
                    Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    Err(RecvTimeoutError::Timeout) => break,
                }
            }
            changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str, name: &str) -> Input {
        Input {
            id: id.into(),
            name: name.into(),
        }
    }

    fn inputs() -> Inputs {
        Inputs {
            devices: vec![
                input("BuiltInMicrophoneDevice", "MacBook Pro Microphone"),
                input("usb-1", "Shure MV7"),
            ],
            default: Some("BuiltInMicrophoneDevice".into()),
        }
    }

    fn airpods() -> Choice {
        Choice {
            id: "airpods".into(),
            name: "AirPods Pro".into(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn settings_meter_never_displays_an_old_sample_as_live_input() {
        use std::sync::{atomic::AtomicU32, Arc, Mutex};
        let (stop, _receiver) = std::sync::mpsc::channel();
        let arrival = Arc::new(Mutex::new(Some(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        )));
        let meter = Meter {
            stop,
            level: Arc::new(AtomicU32::new(0.7_f32.to_bits())),
            arrival: arrival.clone(),
        };
        assert_eq!(meter.level(), 0.0);
        *arrival.lock().unwrap() = Some(std::time::Instant::now());
        assert!(meter.level() > 0.6);
    }

    #[test]
    fn a_missing_device_falls_back_to_the_default_and_says_so() {
        let inputs = inputs();
        let built_in = &inputs.devices[0];
        assert_eq!(Pick::new(None, &inputs), Pick::Default(Some(built_in)));
        assert_eq!(Pick::new(None, &inputs).note(), None);

        let shure = Choice {
            id: "usb-1".into(),
            name: "Old name".into(),
        };
        let pick = Pick::new(Some(&shure), &inputs);
        assert_eq!(pick, Pick::Chosen(&inputs.devices[1]));
        assert_eq!(pick.device().unwrap().name, "Shure MV7");

        let wanted = airpods();
        let pick = Pick::new(Some(&wanted), &inputs);
        assert_eq!(pick.device(), Some(built_in));
        assert_eq!(
            pick.note().unwrap(),
            "AirPods Pro isn't connected. Using MacBook Pro Microphone."
        );

        let none = Inputs::default();
        assert_eq!(Pick::new(None, &none).device(), None);
        assert_eq!(
            Pick::new(Some(&wanted), &none).note().unwrap(),
            "AirPods Pro isn't connected, and there's no other microphone."
        );
    }

    #[test]
    fn the_choice_round_trips_through_the_config_keeping_other_keys() {
        let dir = std::env::temp_dir().join(format!(
            "apparent-mic-{}-{}",
            std::process::id(),
            crate::shared::clock::now_epoch_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"ingestToken":"secret"}"#).unwrap();
        let inputs = inputs();
        assert_eq!(choice_in(&path), None);

        save_in(&path, "usb-1", &inputs).unwrap();
        assert_eq!(
            choice_in(&path),
            Some(Choice {
                id: "usb-1".into(),
                name: "Shure MV7".into()
            })
        );
        save_in(&path, "airpods", &inputs).unwrap();
        assert_eq!(
            choice_in(&path).unwrap().id,
            "usb-1",
            "can't name it, so no change"
        );
        save_in(&path, "default", &inputs).unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config.get("microphone"), None);
        assert_eq!(config["ingestToken"], "secret");
        assert_eq!(choice_in(&path), None);

        std::fs::write(&path, r#"{"microphone":{"id":"","name":"x"}}"#).unwrap();
        assert_eq!(choice_in(&path), None, "an empty id is the default");
        std::fs::write(&path, r#"{"microphone":42,"ingestToken":"kept"}"#).unwrap();
        assert_eq!(choice_in(&path), None);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap()["ingestToken"],
            "kept",
            "a bad microphone value never drops the rest"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// This preference is sampled at capture admission, never inferred from silence.
pub fn enabled() -> bool { enabled_in(&crate::shared::handoff::default_config_path()) }
pub fn enabled_in(path: &std::path::Path) -> bool {
    crate::shared::handoff::load_config_from(path).microphone_enabled.unwrap_or(true)
}
