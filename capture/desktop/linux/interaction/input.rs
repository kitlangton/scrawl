//! Keys and mouse buttons straight from the kernel's input devices (`/dev/input/event*`), for a
//! Wayland session, where no app hears another's input. Reading them needs membership of the
//! `input` group (`sudo usermod -aG input $USER`, then log in again): the same trust macOS asks
//! for with Accessibility. Apparent only keeps which modifiers and buttons are down and a few
//! keys (its shortcuts, Esc, Ctrl+Z); it never records what is typed.
//!
//! On X11 none of this is used: the X server answers the same questions (`super::x11`).

use std::collections::{HashSet, VecDeque};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Linux input event codes (`linux/input-event-codes.h`).
pub mod code {
    pub const KEY_ESC: u16 = 1;
    pub const KEY_LEFTCTRL: u16 = 29;
    pub const KEY_LEFTSHIFT: u16 = 42;
    pub const KEY_RIGHTSHIFT: u16 = 54;
    pub const KEY_LEFTALT: u16 = 56;
    pub const KEY_RIGHTCTRL: u16 = 97;
    pub const KEY_RIGHTALT: u16 = 100;
    pub const KEY_LEFTMETA: u16 = 125;
    pub const KEY_RIGHTMETA: u16 = 126;
    pub const BTN_LEFT: u16 = 0x110;
    pub const BTN_RIGHT: u16 = 0x111;
    pub const BTN_TOOL_FINGER: u16 = 0x145;
    pub const BTN_TOUCH: u16 = 0x14a;
}

const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;

/// What is held down right now, across every keyboard and mouse.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub keys: HashSet<u16>,
    /// Where an absolute pointing device (a tablet, a touchscreen, a virtual machine's or remote
    /// desktop's pointer) last put the pointer, 0–1 across the desktop; `None` once a mouse or
    /// touchpad has moved it since.
    pub absolute: Option<[f64; 2]>,
    /// Ordered native button edges. Cross-device modifier order is qualified
    /// separately from per-edge timestamp provenance.
    /// Consumers reject pre-capture events and handle `button_events_lost` before replay.
    /// Only left/right buttons are retained, never ordinary typed keys.
    pub button_events: Vec<ButtonEvent>,
    /// At least one edge was lost; consumers must discard any pending gesture.
    pub button_events_lost: bool,
}

#[derive(Clone, Debug)]
/// A native button transition, with modifiers captured while applying that event.
/// This contains no position: a later cursor observation cannot locate this edge.
pub struct ButtonEvent {
    pub at: Instant,
    pub button: u16,
    pub pressed: bool,
    pub modifiers: ButtonModifiers,
    /// True only for a single descriptor. Independent device readiness cannot
    /// establish a watermark, even when all device clocks are monotonic.
    pub modifiers_known: bool,
    /// `evdev_monotonic` when the kernel clock is available, otherwise `observation`.
    pub time_source: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl ButtonModifiers {
    fn from_held(held: &Held) -> Self {
        Self {
            shift: held.shift(),
            ctrl: held.ctrl(),
            alt: held.alt(),
            super_key: held.super_key(),
        }
    }
}

impl Held {
    pub fn any(&self, codes: &[u16]) -> bool {
        codes.iter().any(|c| self.keys.contains(c))
    }
    pub fn shift(&self) -> bool {
        self.any(&[code::KEY_LEFTSHIFT, code::KEY_RIGHTSHIFT])
    }
    pub fn ctrl(&self) -> bool {
        self.any(&[code::KEY_LEFTCTRL, code::KEY_RIGHTCTRL])
    }
    pub fn alt(&self) -> bool {
        self.any(&[code::KEY_LEFTALT, code::KEY_RIGHTALT])
    }
    pub fn super_key(&self) -> bool {
        self.any(&[code::KEY_LEFTMETA, code::KEY_RIGHTMETA])
    }
    pub fn left(&self) -> bool {
        self.keys.contains(&code::BTN_LEFT)
    }
    pub fn right(&self) -> bool {
        self.keys.contains(&code::BTN_RIGHT)
    }
}

struct Reader {
    held: Mutex<Held>,
    /// Keys and buttons that went down since the poller last looked (`take_held`): a click
    /// can be over before the next poll.
    pressed: Mutex<HashSet<u16>>,
}

static READER: OnceLock<Reader> = OnceLock::new();
static STARTED: Mutex<bool> = Mutex::new(false);

fn event_devices() -> Vec<std::path::PathBuf> {
    let mut list: Vec<_> = std::fs::read_dir("/dev/input")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("event"))
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort();
    list
}

fn open(path: &std::path::Path) -> Option<OwnedFd> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .ok()
        .map(OwnedFd::from)
}

/// `EVIOCGBIT(EV_KEY, len)`: which keys a device has.
fn key_bits(fd: &OwnedFd) -> Option<Vec<u8>> {
    const LEN: usize = 96;
    let mut bits = vec![0_u8; LEN];
    let request = (2_u64 << 30) | ((LEN as u64) << 16) | (u64::from(b'E') << 8) | (0x20 + 1);
    // SAFETY: the kernel writes at most `LEN` bytes into `bits`.
    let n = unsafe { libc::ioctl(fd.as_raw_fd(), request as _, bits.as_mut_ptr()) };
    (n > 0).then_some(bits)
}

fn has_key(bits: &[u8], code: u16) -> bool {
    bits.get(usize::from(code / 8))
        .is_some_and(|b| b & (1 << (code % 8)) != 0)
}

/// Keyboards and mice: devices with Shift or a left button.
fn interesting(fd: &OwnedFd) -> bool {
    key_bits(fd)
        .is_some_and(|bits| has_key(&bits, code::KEY_LEFTSHIFT) || has_key(&bits, code::BTN_LEFT))
}

/// `EVIOCGABS(axis)`: an absolute axis's range, `None` when the device has none.
fn abs_range(fd: &OwnedFd, axis: u16) -> Option<(i32, i32)> {
    // struct input_absinfo: value, minimum, maximum, fuzz, flat, resolution.
    let mut info = [0_i32; 6];
    let request = (2_u64 << 30) | (24 << 16) | (u64::from(b'E') << 8) | (0x40 + u64::from(axis));
    // SAFETY: the kernel writes one `input_absinfo` (24 bytes) into `info`.
    let n = unsafe { libc::ioctl(fd.as_raw_fd(), request as _, info.as_mut_ptr()) };
    (n >= 0 && info[2] > info[1]).then_some((info[1], info[2]))
}

/// A device that points at absolute positions (not a touchpad, whose axes are the pad's).
fn absolute_axes(fd: &OwnedFd) -> Option<[(i32, i32); 2]> {
    let bits = key_bits(fd)?;
    if has_key(&bits, code::BTN_TOOL_FINGER)
        || !(has_key(&bits, code::BTN_LEFT) || has_key(&bits, code::BTN_TOUCH))
    {
        return None;
    }
    Some([abs_range(fd, ABS_X)?, abs_range(fd, ABS_Y)?])
}

/// Whether Apparent may read at least one keyboard or mouse.
pub fn devices_readable() -> bool {
    event_devices()
        .iter()
        .filter_map(|p| open(p))
        .any(|fd| interesting(&fd))
}

/// Starts reading every keyboard and mouse (once). `false` when none can be read.
pub fn start() -> bool {
    let mut started = STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *started {
        return true;
    }
    let fds: Vec<OwnedFd> = event_devices()
        .iter()
        .filter_map(|p| open(p))
        .filter(interesting)
        .collect();
    if fds.is_empty() {
        return false;
    }
    crate::capture::engine::runtime::apparent_log(format!("input devices: reading {}", fds.len()));
    READER.get_or_init(|| Reader {
        held: Mutex::new(Held::default()),
        pressed: Mutex::new(HashSet::new()),
    });
    *started = std::thread::Builder::new()
        .name("apparent-evdev".into())
        .spawn(move || read_loop(fds))
        .is_ok();
    *started
}

/// Where an absolute pointing device put the pointer, 0–1 across the desktop, unless a mouse
/// has moved it since.
pub fn absolute_pointer() -> Option<[f64; 2]> {
    held()?.absolute
}

/// What is held, counting what was pressed and let go since the last call, for the one poller.
pub fn take_held() -> Option<Held> {
    READER.get().map(take_held_from)
}

fn take_held_from(reader: &Reader) -> Held {
    let mut state = reader
        .held
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let events = std::mem::take(&mut state.button_events);
    let lost = std::mem::take(&mut state.button_events_lost);
    let mut held = state.clone();
    held.button_events = events;
    held.button_events_lost = lost;
    held.keys.extend(
        reader
            .pressed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(),
    );
    held
}

/// What is held, when the devices are being read.
pub fn held() -> Option<Held> {
    READER.get().map(|r| {
        r.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    })
}

fn event_instant(event: &[u8], clock: libc::clockid_t) -> (Instant, &'static str) {
    let received = Instant::now();
    let seconds = i64::from_ne_bytes(event[0..8].try_into().unwrap());
    let micros = i64::from_ne_bytes(event[8..16].try_into().unwrap());
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    // SAFETY: now is writable for one timespec; clock is a supported evdev clock.
    if clock != libc::CLOCK_MONOTONIC || unsafe { libc::clock_gettime(clock, &mut now) } != 0 {
        return (received, "observation");
    }
    match event_age(now.tv_sec, now.tv_nsec, seconds, micros)
        .and_then(|age| received.checked_sub(age))
    {
        Some(at) => (at, "evdev_monotonic"),
        None => (received, "observation"),
    }
}

fn event_age(now_seconds: i64, now_nanos: i64, seconds: i64, micros: i64) -> Option<Duration> {
    if seconds < 0
        || now_seconds < 0
        || !(0..1_000_000).contains(&micros)
        || !(0..1_000_000_000).contains(&now_nanos)
    {
        return None;
    }
    Duration::new(now_seconds as u64, now_nanos as u32)
        .checked_sub(Duration::new(seconds as u64, micros as u32 * 1000))
}

fn read_loop(fds: Vec<OwnedFd>) {
    // struct input_event on 64-bit Linux: timeval (16), type (2), code (2), value (4).
    // evdev defaults to realtime; request monotonic timestamps to avoid wall-clock
    // adjustments. Preserve the default clock when an older device rejects it.
    let clocks: Vec<_> = fds
        .iter()
        .map(|fd| {
            let clock = libc::CLOCK_MONOTONIC;
            // SAFETY: EVIOCSCLOCKID reads one initialized clockid_t from this pointer.
            if unsafe { libc::ioctl(fd.as_raw_fd(), 0x400445a0_u64 as libc::c_ulong, &clock) } == 0
            {
                clock
            } else {
                libc::CLOCK_REALTIME
            }
        })
        .collect();
    let axes: Vec<Option<[(i32, i32); 2]>> = fds.iter().map(absolute_axes).collect();
    let mut polls: Vec<libc::pollfd> = fds
        .iter()
        .map(|fd| libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let mut pending = ReadBuffers::new(&clocks);
    loop {
        // SAFETY: `polls` outlives the call and its length is passed with it.
        let timeout = if pending.queues.iter().all(VecDeque::is_empty) {
            1_000
        } else {
            0
        };
        let ready = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, timeout) };
        if ready < 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let events = read_ready_events(&mut polls, &axes, &clocks, &mut pending);
        if let Some(reader) = READER.get() {
            apply_events(reader, events);
        }
    }
}

struct ReadBuffers {
    queues: Vec<VecDeque<PendingEvent>>,
    ordering_known: bool,
}

impl ReadBuffers {
    fn new(clocks: &[libc::clockid_t]) -> Self {
        Self {
            queues: (0..clocks.len()).map(|_| VecDeque::new()).collect(),
            ordering_known: clocks.len() == 1,
        }
    }
}

// Keep a bounded lookahead per descriptor. A capped batch retains unread heads
// for the next pass rather than applying a later keyboard release ahead of an
// earlier mouse edge still buffered in another descriptor.
fn read_ready_events(
    polls: &mut [libc::pollfd],
    axes: &[Option<[(i32, i32); 2]>],
    clocks: &[libc::clockid_t],
    pending: &mut ReadBuffers,
) -> Vec<PendingEvent> {
    const SIZE: usize = 24;
    const MAX_BATCH: usize = 4096;
    let mut buffer = [0_u8; SIZE * 64];
    let mut events = Vec::new();
    while events.len() < MAX_BATCH {
        for (index, poll) in polls.iter_mut().enumerate() {
            if !pending.queues[index].is_empty() || poll.revents & libc::POLLIN == 0 {
                continue;
            }
            // SAFETY: buffer is writable for its entire passed length.
            let n = unsafe { libc::read(poll.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if n <= 0 {
                poll.revents &= !libc::POLLIN;
                continue;
            }
            for event in buffer[..n as usize].chunks_exact(SIZE) {
                let (at, time_source) = event_instant(event, clocks[index]);
                if clocks.len() > 1 && time_source != "evdev_monotonic" {
                    pending.ordering_known = false;
                }
                pending.queues[index].push_back(PendingEvent {
                    kind: u16::from_ne_bytes(event[16..18].try_into().unwrap()),
                    code: u16::from_ne_bytes(event[18..20].try_into().unwrap()),
                    value: i32::from_ne_bytes(event[20..24].try_into().unwrap()),
                    axes: axes[index],
                    at,
                    time_source,
                    modifiers_known: true,
                });
            }
        }
        let next = pending
            .queues
            .iter()
            .enumerate()
            .filter_map(|(index, queue)| queue.front().map(|event| (index, event.at)))
            .min_by_key(|&(_, at)| at)
            .map(|(index, _)| index);
        let Some(index) = next else {
            break;
        };
        let mut event = pending.queues[index].pop_front().unwrap();
        event.modifiers_known = pending.ordering_known;
        events.push(event);
    }
    events
}

struct PendingEvent {
    kind: u16,
    code: u16,
    value: i32,
    axes: Option<[(i32, i32); 2]>,
    at: Instant,
    time_source: &'static str,
    modifiers_known: bool,
}

fn apply_events(reader: &Reader, events: Vec<PendingEvent>) {
    // read_ready_events already merges descriptor heads, retaining native order
    // within each device. Mixed clock snapshots explicitly remain unqualified.
    let mut held = reader
        .held
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut pressed = reader
        .pressed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for event in events {
        let PendingEvent {
            kind,
            code,
            value,
            axes,
            at,
            time_source,
            modifiers_known,
        } = event;
        match (kind, axes) {
            (EV_KEY, _) if value == 0 || value == 1 => {
                if value == 1 {
                    held.keys.insert(code);
                    pressed.insert(code);
                } else {
                    held.keys.remove(&code);
                }
                if matches!(code, code::BTN_LEFT | code::BTN_RIGHT) {
                    if held.button_events.len() == 512 {
                        held.button_events.remove(0);
                        held.button_events_lost = true;
                        crate::capture::engine::runtime::apparent_log(
                            "input button queue overflow",
                        );
                    }
                    let modifiers = ButtonModifiers::from_held(&held);
                    held.button_events.push(ButtonEvent {
                        at,
                        time_source,
                        button: code,
                        pressed: value == 1,
                        modifiers,
                        modifiers_known,
                    });
                }
            }
            (EV_ABS, Some(ranges)) if code == ABS_X || code == ABS_Y => {
                let (min, max) = ranges[usize::from(code)];
                let at = f64::from(value - min) / f64::from(max - min);
                let mut point = held.absolute.unwrap_or([0.5, 0.5]);
                point[usize::from(code)] = at.clamp(0.0, 1.0);
                held.absolute = Some(point);
            }
            (EV_REL, _) if code == REL_X || code == REL_Y => held.absolute = None,
            _ => {}
        }
    }
}

/// The evdev codes of A–Z, on a US layout's positions.
const LETTERS: [u16; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];

/// The Windows-style virtual key (what Apparent stores shortcuts as on Linux) for an evdev code:
/// letters, digits and F1–F12 on a US layout's positions, plus Space, Enter, Tab, Backspace,
/// Delete and Esc.
pub fn vk_for_evdev_code(code: u16) -> Option<u16> {
    if let Some(i) = LETTERS.iter().position(|&c| c == code) {
        return Some(0x41 + i as u16);
    }
    Some(match code {
        2..=10 => 0x31 + code - 2,
        11 => 0x30,
        59..=68 => 0x70 + code - 59,
        87 => 0x7A,
        88 => 0x7B,
        57 => 0x20,
        28 => 0x0D,
        15 => 0x09,
        14 => 0x08,
        111 => 0x2E,
        code::KEY_ESC => 0x1B,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader() -> Reader {
        Reader {
            held: Mutex::new(Held::default()),
            pressed: Mutex::new(HashSet::new()),
        }
    }

    fn key(code: u16, value: i32, at: Duration) -> [u8; 24] {
        let mut event = [0_u8; 24];
        event[..8].copy_from_slice(&(at.as_secs() as i64).to_ne_bytes());
        event[8..16].copy_from_slice(&(at.subsec_micros() as i64).to_ne_bytes());
        event[16..18].copy_from_slice(&EV_KEY.to_ne_bytes());
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        event[20..].copy_from_slice(&value.to_ne_bytes());
        event
    }

    fn replay(descriptors: &[Vec<[u8; 24]>], clock: libc::clockid_t) -> Vec<PendingEvent> {
        replay_batches(descriptors, &vec![clock; descriptors.len()])
            .into_iter()
            .flatten()
            .collect()
    }

    fn replay_batches(
        descriptors: &[Vec<[u8; 24]>],
        clocks: &[libc::clockid_t],
    ) -> Vec<Vec<PendingEvent>> {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let pairs: Vec<_> = descriptors
            .iter()
            .map(|events| {
                let (read, mut write) = UnixStream::pair().unwrap();
                read.set_nonblocking(true).unwrap();
                let bytes: Vec<_> = events.iter().flatten().copied().collect();
                write.write_all(&bytes).unwrap();
                (read, write)
            })
            .collect();
        let mut polls: Vec<_> = pairs
            .iter()
            .map(|(read, _)| libc::pollfd {
                fd: read.as_raw_fd(),
                events: libc::POLLIN,
                revents: libc::POLLIN,
            })
            .collect();
        let mut pending = ReadBuffers::new(clocks);
        let mut batches = Vec::new();
        loop {
            let batch =
                read_ready_events(&mut polls, &vec![None; pairs.len()], clocks, &mut pending);
            if batch.is_empty() {
                break;
            }
            batches.push(batch);
        }
        batches
    }

    #[test]
    fn delayed_poll_retains_both_button_edges() {
        let reader = reader();
        let event = |code, value| key(code, value, Duration::ZERO);
        apply_events(
            &reader,
            replay(&[vec![event(code::KEY_LEFTSHIFT, 1)]], libc::CLOCK_REALTIME),
        );
        assert!(take_held_from(&reader).shift());
        // The physical sequence completes while the consumer is busy.
        apply_events(
            &reader,
            replay(
                &[vec![
                    event(code::BTN_LEFT, 1),
                    event(code::BTN_LEFT, 0),
                    event(code::KEY_LEFTSHIFT, 0),
                ]],
                libc::CLOCK_REALTIME,
            ),
        );
        let snapshot = take_held_from(&reader);
        assert!(
            !snapshot.shift(),
            "current state correctly sees Shift released"
        );
        assert_eq!(
            snapshot.button_events.len(),
            2,
            "both historical edges must survive the delayed poll"
        );
        assert!(
            snapshot
                .button_events
                .iter()
                .all(|event| event.modifiers.shift)
        );
        assert!(snapshot.button_events[0].pressed);
        assert!(!snapshot.button_events[1].pressed);
        assert!(take_held_from(&reader).button_events.is_empty());
    }

    #[test]
    fn cross_device_batch_uses_event_time_before_descriptor_order() {
        let mut now: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: now is writable for one timespec.
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) },
            0
        );
        let base = Duration::new(now.tv_sec as u64, now.tv_nsec as u32) - Duration::from_secs(2);
        for shift in [code::KEY_LEFTSHIFT, code::KEY_RIGHTSHIFT] {
            let reader = reader();
            let event = |code, value, ms| key(code, value, base + Duration::from_millis(ms));
            // Keyboard descriptor is first, but its release happened after the mouse click.
            let descriptors = vec![
                vec![event(shift, 1, 0), event(shift, 0, 887)],
                vec![event(code::BTN_LEFT, 1, 619), event(code::BTN_LEFT, 0, 755)],
            ];
            apply_events(&reader, replay(&descriptors, libc::CLOCK_MONOTONIC));
            assert!(!reader.held.lock().unwrap().shift());
            let snapshot = take_held_from(&reader);
            assert_eq!(snapshot.button_events.len(), 2);
            assert!(
                snapshot
                    .button_events
                    .iter()
                    .all(|event| event.modifiers.shift && event.time_source == "evdev_monotonic")
            );
            assert!(snapshot.button_events[0].at < snapshot.button_events[1].at);
            assert!(snapshot.button_events[0].at.elapsed() > Duration::from_secs(1));
        }
    }

    fn monotonic_base() -> Duration {
        let mut now: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: now is writable for one timespec.
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) },
            0
        );
        Duration::new(now.tv_sec as u64, now.tv_nsec as u32) - Duration::from_secs(2)
    }

    #[test]
    fn capped_read_keeps_cross_device_lookahead_without_losing_or_reordering_edges() {
        let base = monotonic_base();
        let event = |code, value, ms| key(code, value, base + Duration::from_millis(ms));
        let mut keyboard = vec![event(code::KEY_LEFTSHIFT, 1, 0)];
        keyboard.extend((0..4100).map(|_| event(code::KEY_LEFTSHIFT, 2, 600)));
        keyboard.push(event(code::KEY_LEFTSHIFT, 0, 887));
        let mouse = vec![event(code::BTN_LEFT, 1, 619), event(code::BTN_LEFT, 0, 755)];
        let batches = replay_batches(&[keyboard, mouse], &[libc::CLOCK_MONOTONIC; 2]);
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 4104);
        assert_eq!(batches[0].len(), 4096);
        assert!(batches.iter().all(|batch| batch.len() <= 4096));
        let reader = reader();
        for batch in batches {
            apply_events(&reader, batch);
        }
        let snapshot = take_held_from(&reader);
        assert!(!snapshot.button_events_lost);
        assert_eq!(snapshot.button_events.len(), 2);
        assert!(
            snapshot
                .button_events
                .iter()
                .all(|event| event.modifiers.shift && !event.modifiers_known)
        );
        assert!(!reader.held.lock().unwrap().shift());
    }

    #[test]
    fn exact_batch_boundary_does_not_report_a_loss() {
        let events = vec![key(code::KEY_LEFTSHIFT, 2, Duration::ZERO); 4096];
        let batches = replay_batches(&[events], &[libc::CLOCK_REALTIME]);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 4096);
        let reader = reader();
        for batch in batches {
            apply_events(&reader, batch);
        }
        assert!(!take_held_from(&reader).button_events_lost);
    }

    #[test]
    fn late_modifier_readiness_never_qualifies_an_earlier_button_snapshot() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (keyboard, mut keyboard_writer) = UnixStream::pair().unwrap();
        let (mouse, mut mouse_writer) = UnixStream::pair().unwrap();
        keyboard.set_nonblocking(true).unwrap();
        mouse.set_nonblocking(true).unwrap();
        let base = monotonic_base();
        let event = |code, value, ms| key(code, value, base + Duration::from_millis(ms));
        let mut polls = [&keyboard, &mouse]
            .iter()
            .map(|read| libc::pollfd {
                fd: read.as_raw_fd(),
                events: libc::POLLIN,
                revents: libc::POLLIN,
            })
            .collect::<Vec<_>>();
        // Keep both streams alive while their readiness is independently delayed.
        let _streams = [keyboard, mouse];
        let clocks = [libc::CLOCK_MONOTONIC; 2];
        let mut pending = ReadBuffers::new(&clocks);
        mouse_writer
            .write_all(&event(code::BTN_LEFT, 1, 619))
            .unwrap();
        let reader = reader();
        apply_events(
            &reader,
            read_ready_events(&mut polls, &[None; 2], &clocks, &mut pending),
        );
        let first = take_held_from(&reader);
        assert_eq!(first.button_events.len(), 1);
        assert!(!first.button_events[0].modifiers_known);
        assert!(!first.button_events[0].modifiers.shift);
        // The earlier kernel timestamp does not make this event readable sooner.
        keyboard_writer
            .write_all(&event(code::KEY_LEFTSHIFT, 1, 0))
            .unwrap();
        mouse_writer
            .write_all(&event(code::BTN_LEFT, 0, 755))
            .unwrap();
        polls
            .iter_mut()
            .for_each(|poll| poll.revents = libc::POLLIN);
        apply_events(
            &reader,
            read_ready_events(&mut polls, &[None; 2], &clocks, &mut pending),
        );
        let later = take_held_from(&reader);
        assert_eq!(later.button_events.len(), 1);
        assert!(later.button_events[0].modifiers.shift);
        assert!(!later.button_events[0].modifiers_known);
    }

    #[test]
    fn mixed_device_clocks_do_not_qualify_modifier_order() {
        let base = monotonic_base();
        let event = |code, value, ms| key(code, value, base + Duration::from_millis(ms));
        let descriptors = vec![
            vec![
                event(code::KEY_LEFTSHIFT, 1, 0),
                event(code::KEY_LEFTSHIFT, 0, 887),
            ],
            vec![event(code::BTN_LEFT, 1, 619), event(code::BTN_LEFT, 0, 755)],
        ];
        let reader = reader();
        for batch in replay_batches(&descriptors, &[libc::CLOCK_MONOTONIC, libc::CLOCK_REALTIME]) {
            apply_events(&reader, batch);
        }
        let snapshot = take_held_from(&reader);
        assert_eq!(snapshot.button_events.len(), 2);
        assert!(
            snapshot
                .button_events
                .iter()
                .all(|event| !event.modifiers_known && event.time_source == "observation")
        );
    }

    #[test]
    fn queue_overflow_is_bounded_and_reported_once() {
        let reader = reader();
        let now = Instant::now();
        apply_events(
            &reader,
            (0..514)
                .map(|i| PendingEvent {
                    kind: EV_KEY,
                    code: code::BTN_LEFT,
                    value: i % 2,
                    axes: None,
                    at: now,
                    time_source: "observation",
                    modifiers_known: true,
                })
                .collect(),
        );
        let snapshot = take_held_from(&reader);
        assert_eq!(snapshot.button_events.len(), 512);
        assert!(snapshot.button_events_lost);
        assert!(!take_held_from(&reader).button_events_lost);
    }

    #[test]
    fn invalid_or_unavailable_event_clock_is_explicit_observation_time() {
        let mut event = [0; 24];
        event[..8].copy_from_slice(&i64::MAX.to_ne_bytes());
        let (at, source) = event_instant(&event, libc::CLOCK_MONOTONIC);
        assert_eq!(source, "observation");
        assert!(at.elapsed() < Duration::from_secs(1));
        event[..8].copy_from_slice(&0_i64.to_ne_bytes());
        event[8..16].copy_from_slice(&1_000_000_i64.to_ne_bytes());
        assert_eq!(
            event_instant(&event, libc::CLOCK_MONOTONIC).1,
            "observation"
        );
        assert_eq!(event_instant(&event, libc::CLOCK_REALTIME).1, "observation");
        assert_eq!(event_instant(&event, -1).1, "observation");
    }

    #[test]
    fn event_clock_conversion_retains_age_and_rejects_invalid_time() {
        assert_eq!(
            event_age(100, 500_000_000, 100, 250_000),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            event_age(100, 0, 99, 900_000),
            Some(Duration::from_millis(100))
        );
        assert_eq!(event_age(100, 0, 101, 0), None);
        assert_eq!(event_age(100, 0, 100, 1_000_000), None);
        assert_eq!(event_age(100, 0, -1, 0), None);
    }

    #[test]
    fn evdev_codes_map_to_virtual_keys() {
        assert_eq!(vk_for_evdev_code(31), Some(u16::from(b'S')));
        assert_eq!(vk_for_evdev_code(47), Some(u16::from(b'V')));
        assert_eq!(vk_for_evdev_code(44), Some(u16::from(b'Z')));
        assert_eq!(vk_for_evdev_code(2), Some(u16::from(b'1')));
        assert_eq!(vk_for_evdev_code(11), Some(u16::from(b'0')));
        assert_eq!(vk_for_evdev_code(59), Some(0x70));
        assert_eq!(vk_for_evdev_code(88), Some(0x7B));
        assert_eq!(vk_for_evdev_code(code::KEY_LEFTSHIFT), None);
    }
}
