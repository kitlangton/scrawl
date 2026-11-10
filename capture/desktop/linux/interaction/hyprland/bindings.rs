//! Lua runtime registrations retain exact handles. Hyprland 0.56's remove()
//! removes every binding on a chord; set_enabled(false) changes only its handle.
//! The bounded compositor cache reuses disabled handles across app restarts.
use std::sync::{Mutex, OnceLock, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

use anyhow::{Result, bail, ensure};
use super::{Existing, command, description, existing, external_record, external_record_live,
    lua_chord, lua_str, modmask, request, request_with_timeout};

static SERIAL: Mutex<()> = Mutex::new(());
static CLOSING: AtomicBool = AtomicBool::new(false);
static USED: AtomicBool = AtomicBool::new(false);

fn owner() -> &'static str {
    static OWNER: OnceLock<String> = OnceLock::new();
    OWNER.get_or_init(|| {
        let mut clock = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // Shared monotonic clock for compositor-local incarnation ordering;
        // fixed width preserves exact order without Lua number rounding.
        assert_eq!(unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) }, 0);
        format!("{:020}:{:09}:{:010}", clock.tv_sec, clock.tv_nsec, std::process::id())
    })
}

// This is a resource namespace, not authentication against the compositor's user.
// Only handles returned by our hl.bind call enter it. No description claims a
// user/permanent binding. Reload expires handles; pruning recovers their slots.
const CACHE: &str = r#"
local name = '__apparent_shortcut_handles_v1'
local r = _G[name]
if r == nil then r = { version = 1, entries = {} }; _G[name] = r end
assert(type(r) == 'table' and r.version == 1 and type(r.entries) == 'table', 'Apparent shortcut namespace changed')
local count = 0
for key, entry in pairs(r.entries) do
  if entry.handle:is_enabled() == nil then r.entries[key] = nil
  else count = count + 1 end
end
assert(count <= 64, 'Apparent shortcut cache changed')
"#;

fn admit(owner: &str) -> String {
    format!("local owner = {}\n\
        assert(r.latest == nil or r.latest < owner or (r.latest == owner and not r.closed), 'Stale Apparent shortcut session')\n\
        if r.latest ~= owner then\n\
          for _, old in pairs(r.entries) do\n\
            if old.owner ~= nil then old.handle:set_enabled(false); old.owner = nil end\n\
          end\n\
          r.latest = owner; r.closed = false\n\
        end\n", lua_str(owner))
}

fn disable(owner: &str, id: Option<&str>) -> String {
    let (prefix, which) = if let Some(id) = id {
        (admit(owner), format!("entry.owner == {} and entry.id == {}", lua_str(owner), lua_str(id)))
    } else {
        (format!("if r.latest == nil or r.latest <= {0} then r.latest = {0}; r.closed = true end\n", lua_str(owner)),
         format!("entry.owner ~= nil and entry.owner <= {}", lua_str(owner)))
    };
    format!("eval {CACHE}\n{prefix}for _, entry in pairs(r.entries) do\n\
        if {which} then\n\
          entry.handle:set_enabled(false)\n\
          assert(entry.handle:is_enabled() == false, 'Apparent shortcut still enabled')\n\
          entry.owner = nil\n\
        end\nend")
}

fn enable(owner: &str, id: &str, chord: &str, command: &str, reuse_required: bool) -> String {
    let key = lua_str(&format!("{id}:{chord}"));
    let admission = admit(owner);
    format!("eval {CACHE}\n{admission}\n\
        local key = {key}\nlocal entry = r.entries[key]\n\
        assert(entry ~= nil or not {reuse_required}, 'Apparent shortcut handle unavailable; reload desktop configuration')\n\
        if entry == nil then\n\
          assert(count < 64, 'Apparent shortcut cache full; reload desktop configuration')\n\
          entry = {{ id = {}, command = {} }}\n\
          entry.handle = hl.bind({}, function()\n\
            if entry.owner ~= nil then hl.dispatch(hl.dsp.exec_cmd(entry.command)) end\n\
          end, {{ description = {} }})\n\
          r.entries[key] = entry\n\
        end\n\
        entry.command = {}\nentry.owner = owner\n\
        entry.handle:set_enabled(true)\n\
        assert(entry.handle:is_enabled() == true, 'Apparent shortcut unavailable')",
        lua_str(id), lua_str(command), lua_str(chord), lua_str(&managed_description(id)),
        lua_str(command))
}

pub(super) fn managed_description(id: &str) -> String {
    format!("{} [managed-v1]", description(id))
}

fn run(command: &str, timeout: Duration) -> Result<()> {
    let reply = request_with_timeout(command, timeout)?;
    ensure!(reply.trim() == "ok", "Hyprland shortcut: {}", reply.trim());
    Ok(())
}

pub(super) fn bind(id: &'static str, chord: Option<(u32, &str)>) -> Result<()> {
    ensure!(matches!(id, "record" | "paste"), "Unknown shortcut");
    // Pair with close's two-flag check: sequential consistency prevents both
    // threads from missing the other's publication in a first-bind/Quit race.
    USED.store(true, Ordering::SeqCst);
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if CLOSING.load(Ordering::SeqCst) { return Ok(()); }
    run(&disable(owner(), Some(id)), Duration::from_millis(200))?;
    let Some((modifiers, key)) = chord else { return Ok(()); };
    if key == "Copilot" && id == "record"
        && external_record().is_some_and(|record| record.carbon_modifiers == modifiers)
    {
        ensure!(external_record_live().is_some(), "Hyprland-managed Record shortcut changed; restart Apparent");
        return Ok(());
    }
    let chord = lua_chord(modmask(modifiers), key);
    let reuse = match existing(&request("j/binds")?, modmask(modifiers), key, id) {
        Existing::Apparent => return Ok(()), // Unowned/permanent: never claim its handle.
        Existing::Other => bail!("{chord} is already bound or cannot be observed"),
        Existing::Managed => true,
        Existing::Free => false,
    };
    if CLOSING.load(Ordering::SeqCst) { return Ok(()); }
    let result = run(&enable(owner(), id, &chord, &command(id)?, reuse), Duration::from_millis(200));
    // A reply can be lost after registration. The compositor retains the exact
    // handle/owner even then, so Quit or a later pause can still disable it.
    if CLOSING.load(Ordering::SeqCst) {
        let _ = run(&disable(owner(), None), Duration::from_millis(80));
    }
    result
}

/// No synchronous compositor I/O on GPUI's quit callback. GPUI waits at most
/// 100 ms for returned futures; the worker uses at most 80 ms socket timeouts.
pub(super) fn close() -> impl std::future::Future<Output = ()> + 'static {
    CLOSING.store(true, Ordering::SeqCst);
    let (tx, rx) = flume::bounded(1);
    let deadline = Instant::now() + Duration::from_millis(80);
    let worker = std::thread::Builder::new().name("apparent-shortcut-close".into()).spawn(move || {
        // Safe on every desktop. Admission was closed before this read; a late
        // first bind sees CLOSING and cannot create a handle after we skip I/O.
        if !USED.load(Ordering::SeqCst) { let _ = tx.send(()); return; }
        let result = loop {
            match SERIAL.try_lock() {
                Ok(_guard) => {
                    break deadline.checked_duration_since(Instant::now())
                        .filter(|remaining| !remaining.is_zero())
                        .ok_or_else(|| anyhow::anyhow!("Shortcut cleanup deadline reached"))
                        .and_then(|remaining| run(&disable(owner(), None), remaining));
                }
                Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline =>
                    std::thread::sleep(Duration::from_millis(1)),
                _ => break Err(anyhow::anyhow!("Shortcut cleanup unavailable")),
            }
        };
        if let Err(error) = result {
            crate::capture::engine::runtime::apparent_log(format!(
                "Hyprland runtime shortcuts may remain until configuration reload: {error:#}"));
        }
        let _ = tx.send(());
    });
    if let Err(error) = worker {
        crate::capture::engine::runtime::apparent_log(format!("Hyprland shortcut cleanup could not start: {error}"));
    }
    async move { let _ = rx.recv_async().await; }
}

#[cfg(test)]
mod tests;
