//! Plasma 5's GlobalShortcuts portal doesn't bind new shortcuts. Use its native shortcut
//! service instead, so the chord is consumed rather than also reaching the focused app.
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

use crate::capture::{ShortcutConfig, ShortcutKind};

const COMPONENT: &str = "dev.kitlangton.scrawl";
static BUS: OnceLock<zbus::blocking::Connection> = OnceLock::new();

fn action(id: &str) -> [&str; 4] {
    [
        COMPONENT,
        id,
        "Scrawl",
        if id == "record" {
            "Record or finish"
        } else {
            "Paste latest capture"
        },
    ]
}

fn key(cfg: &ShortcutConfig) -> Option<i32> {
    use crate::capture::{CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY};
    let mut key = match cfg.key_code {
        0x30..=0x5a | 0x20 => cfg.key_code as i32,
        0x70..=0x87 => 0x01000030 + cfg.key_code as i32 - 0x70,
        _ => return None,
    };
    for (carbon, qt) in [
        (CARBON_SHIFT_KEY, 0x02000000),
        (CARBON_CONTROL_KEY, 0x04000000),
        (CARBON_OPTION_KEY, 0x08000000),
        (CARBON_CMD_KEY, 0x10000000),
    ] {
        if cfg.carbon_modifiers & carbon != 0 {
            key |= qt;
        }
    }
    Some(key)
}

/// The chords to request: none while paused or for a shortcut that is turned off.
fn requested_keys(cfg: &ShortcutConfig, paused: bool) -> Result<Vec<i32>> {
    if paused || cfg.is_unset() {
        return Ok(Vec::new());
    }
    Ok(vec![key(cfg).context("Unsupported key")?])
}

pub fn bind(
    record: &ShortcutConfig,
    paste: &ShortcutConfig,
    paused: bool,
) -> Result<(), ShortcutKind> {
    let bind_one = |id: &str, cfg: &ShortcutConfig| -> Result<()> {
        let bus = BUS.get().context("KGlobalAccel is not connected")?;
        let proxy = zbus::blocking::Proxy::new(
            bus,
            "org.kde.kglobalaccel",
            "/kglobalaccel",
            "org.kde.KGlobalAccel",
        )?;
        let action = action(id).to_vec();
        proxy.call_method("doRegister", &(&action,))?;
        let requested = requested_keys(cfg, paused)?;
        // SetPresent | NoAutoloading: Scrawl's configured chord, active while the bus is alive.
        let bound: Vec<i32> = proxy.call("setShortcut", &(&action, &requested, 6_u32))?;
        if bound != requested {
            bail!("Shortcut is already taken");
        }
        Ok(())
    };
    for (id, cfg, kind) in [
        ("record", record, ShortcutKind::Record),
        ("paste", paste, ShortcutKind::Paste),
    ] {
        if let Err(error) = bind_one(id, cfg) {
            crate::capture::scrawl_log(format!("KGlobalAccel {id}: {error:#}"));
            return Err(kind);
        }
    }
    Ok(())
}

pub fn listen(
    record: ShortcutConfig,
    paste: ShortcutConfig,
    pressed: impl Fn(&str) + Send + 'static,
) -> Result<()> {
    let bus = zbus::blocking::Connection::session()?;
    let _ = BUS.set(bus.clone());
    bind(&record, &paste, false)
        .map_err(|kind| anyhow::anyhow!("{kind:?} shortcut was refused"))?;
    let proxy = zbus::blocking::Proxy::new(
        &bus,
        "org.kde.kglobalaccel",
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
    )?;
    let path: zbus::zvariant::OwnedObjectPath = proxy.call("getComponent", &(COMPONENT,))?;
    let (ready_tx, ready_rx) = flume::bounded(1);
    std::thread::Builder::new()
        .name("scrawl-kglobalaccel".into())
        .spawn(move || {
            let run = || -> Result<()> {
                let component = zbus::blocking::Proxy::new(
                    &bus,
                    "org.kde.kglobalaccel",
                    path,
                    "org.kde.kglobalaccel.Component",
                )?;
                let events = component.receive_signal("globalShortcutPressed")?;
                let _ = ready_tx.send(Ok(()));
                for event in events {
                    if let Ok((_, id, _)) = event.body().deserialize::<(String, String, i64)>() {
                        pressed(&id);
                    }
                }
                Ok(())
            };
            if let Err(error) = run() {
                let _ = ready_tx.send(Err(error));
            }
        })?;
    ready_rx.recv_timeout(std::time::Duration::from_secs(5))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_or_paused_shortcut_requests_no_chord() {
        let record = ShortcutConfig::default_record();
        assert!(requested_keys(&ShortcutConfig::unset(), false).unwrap().is_empty());
        assert!(requested_keys(&record, true).unwrap().is_empty());
        assert_eq!(requested_keys(&record, false).unwrap().len(), 1);
    }
}
