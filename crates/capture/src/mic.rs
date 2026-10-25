//! Which microphone Mac captures record from: `"microphone": {"id", "name"}` in
//! `~/.config/scrawl/config.json` (absent = the macOS default, followed as it changes). The id is
//! the CoreAudio device UID, which is also `AVCaptureDevice.uniqueID`; the name is kept only to
//! say which device is missing. A chosen device that isn't connected falls back to the default.

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

/// The input devices connected now, in CoreAudio's order, and the default's id.
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
    /// No choice: the macOS default (none when there's no microphone at all).
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
    choice_in(&crate::handoff::default_config_path())
}

pub fn choice_in(path: &std::path::Path) -> Option<Choice> {
    crate::handoff::load_config_from(path)
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
    crate::handoff::set_config_value_in(path, "microphone", value)
}

/// Enumerates input devices (CoreAudio, a few ms; never on the UI thread).
#[cfg(feature = "mic")]
pub fn inputs() -> Inputs {
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

#[cfg(not(feature = "mic"))]
pub fn inputs() -> Inputs {
    Inputs::default()
}

/// The cpal device for `id`, if connected.
#[cfg(feature = "mic")]
pub fn device(id: &str) -> Option<cpal::Device> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host()
        .input_devices()
        .ok()?
        .find(|device| device.id().is_ok_and(|d| d.id() == id))
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
                crate::log::scrawl_log(format!("Microphone: {note}"));
                *warned = Some(wanted.id.clone());
            }
        }
        _ => *warned = None,
    }
}

/// Calls `changed` on its own thread once at the start, then whenever an input device is plugged
/// in or out or the macOS default changes (bursts coalesced). Stops when dropped.
pub struct Watch {
    #[cfg(target_os = "macos")]
    sender: std::sync::mpsc::Sender<watch::Message>,
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
            std::thread::spawn(changed);
            Self {}
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        let _ = self.sender.send(watch::Message::Stop);
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
            .name("scrawl-mic-watch".into())
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
            crate::log::scrawl_log(format!("Microphone watch failed: {error}"));
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
            "scrawl-mic-{}-{}",
            std::process::id(),
            crate::pipeline::now_epoch_ms()
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
            crate::handoff::load_config_from(&path)
                .ingest_token
                .as_deref(),
            Some("kept"),
            "a bad microphone value never drops the rest"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
