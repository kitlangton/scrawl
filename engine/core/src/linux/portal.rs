//! Existing Wayland ScreenCast execution, extracted from the Desktop portal adapter.
//! Callers retain source choice, consent routing, token persistence and crop policy.
//! These session functions do not enable standalone Capture/2 Wayland support.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::Stream;
use anyhow::{Context, Result, anyhow, bail};

/// `org.gnome.Mutter.ScreenCast.Session.RecordArea`: any rect of the screen (logical pixels,
/// which are XWayland's at scale 1), with the pointer drawn in, no dialog.
pub fn mutter_area(rect: [f64; 4]) -> Result<Stream> {
    let (ready_tx, ready_rx) = flume::bounded::<Result<u32>>(1);
    let (stop_tx, stop_rx) = flume::bounded::<()>(1);
    std::thread::Builder::new()
        .name("apparent-mutter-screencast".into())
        .spawn(move || {
            let run = || -> Result<(zbus::blocking::Connection, zbus::zvariant::OwnedObjectPath, u32)> {
                let bus = zbus::blocking::Connection::session()?;
                let cast = zbus::blocking::Proxy::new(
                    &bus,
                    "org.gnome.Mutter.ScreenCast",
                    "/org/gnome/Mutter/ScreenCast",
                    "org.gnome.Mutter.ScreenCast",
                )?;
                let props: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
                let session_path: zbus::zvariant::OwnedObjectPath =
                    cast.call("CreateSession", &(props,))?;
                let session = zbus::blocking::Proxy::new(
                    &bus,
                    "org.gnome.Mutter.ScreenCast",
                    session_path.clone(),
                    "org.gnome.Mutter.ScreenCast.Session",
                )?;
                let mut props: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
                // 1: the pointer drawn into the frames.
                props.insert("cursor-mode", zbus::zvariant::Value::U32(1));
                props.insert("is-recording", zbus::zvariant::Value::Bool(true));
                let [x, y, w, h] = rect.map(|v| v.round() as i32);
                let stream_path: zbus::zvariant::OwnedObjectPath =
                    session.call("RecordArea", &(x, y, w, h, props))?;
                let stream = zbus::blocking::Proxy::new(
                    &bus,
                    "org.gnome.Mutter.ScreenCast",
                    stream_path,
                    "org.gnome.Mutter.ScreenCast.Stream",
                )?;
                let mut added = stream.receive_signal("PipeWireStreamAdded")?;
                session.call_method("Start", &())?;
                let node: u32 = added
                    .next()
                    .context("No stream")?
                    .body()
                    .deserialize()?;
                Ok((bus, session_path, node))
            };
            match run() {
                Ok((bus, session_path, node)) => {
                    let _ = ready_tx.send(Ok(node));
                    let _ = stop_rx.recv();
                    let _ = bus.call_method(
                        Some("org.gnome.Mutter.ScreenCast"),
                        session_path.as_str(),
                        Some("org.gnome.Mutter.ScreenCast.Session"),
                        "Stop",
                        &(),
                    );
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        })?;
    let node = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| anyhow!("GNOME's screencast didn't answer"))??;
    Ok(Stream::new(node, None, Some(rect), None, Some(stop_tx)))
}

/// Open the existing single-source ScreenCast session using a caller-supplied restore token.
/// The caller retains token storage. The callback reports a request error so the caller can
/// preserve its existing invalid-token policy; cancellation and transport errors do not call it.
/// The returned Stream owns the session/FD until the recorder finishes or drops.
pub fn screencast(
    window: bool,
    restore_token: Option<String>,
    stopping: &AtomicBool,
    request_failed: impl FnOnce(),
) -> Result<(Stream, Option<String>)> {
    screencast_with_control(window, restore_token, stopping, request_failed, || Ok(()))
}

/// Uses the existing consent wait while allowing its trusted caller to refresh
/// durable control into `stopping`. The callback runs on the waiting caller,
/// never on the portal thread; it must remain bounded and does not grant consent.
pub fn screencast_with_control(
    window: bool,
    restore_token: Option<String>,
    stopping: &AtomicBool,
    request_failed: impl FnOnce(),
    observe_control: impl FnMut() -> Result<()>,
) -> Result<(Stream, Option<String>)> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};

    let (ready_tx, ready_rx) = flume::bounded::<
        Result<(
            u32,
            OwnedFd,
            Option<[f64; 4]>,
            Option<String>,
            Option<String>,
        )>,
    >(1);
    let (stop_tx, stop_rx) = flume::bounded::<()>(1);
    std::thread::Builder::new()
        .name("apparent-portal-screencast".into())
        .spawn(move || {
            futures_lite::future::block_on(async move {
                let run = async {
                    let proxy = Screencast::new().await?;
                    let session = proxy.create_session().await?;
                    let request = async {
                        let token = restore_token;
                        let kind = if window {
                            SourceType::Window
                        } else {
                            SourceType::Monitor
                        };
                        proxy
                            .select_sources(
                                &session,
                                CursorMode::Embedded,
                                kind.into(),
                                false,
                                token.as_deref(),
                                // Windows come and go: ask each time for one.
                                if window {
                                    PersistMode::DoNot
                                } else {
                                    PersistMode::ExplicitlyRevoked
                                },
                            )
                            .await?;
                        let streams = proxy.start(&session, None).await?.response()?;
                        let token = streams.restore_token().map(str::to_owned);
                        let stream = streams.streams().first().context("Nothing was shared")?;
                        let rect = match (stream.position(), stream.size()) {
                            (Some((x, y)), Some((w, h))) => {
                                Some([f64::from(x), f64::from(y), f64::from(w), f64::from(h)])
                            }
                            _ => None,
                        };
                        let node = stream.pipe_wire_node_id();
                        let mapping_id = stream.mapping_id().map(str::to_owned);
                        let fd = proxy.open_pipe_wire_remote(&session).await?;
                        anyhow::Ok((node, fd, rect, mapping_id, token))
                    };
                    let cancelled = async {
                        let _ = stop_rx.recv_async().await;
                        bail!("Screen sharing was cancelled")
                    };
                    match futures_lite::future::race(request, cancelled).await {
                        Ok((node, fd, rect, mapping_id, token)) => {
                            anyhow::Ok((proxy, session, node, fd, rect, mapping_id, token))
                        }
                        Err(error) => {
                            let _ = session.close().await;
                            Err(error)
                        }
                    }
                };
                match run.await {
                    Ok((_proxy, session, node, fd, rect, mapping_id, token)) => {
                        let _ = ready_tx.send(Ok((node, fd, rect, mapping_id, token)));
                        let _ = stop_rx.recv_async().await;
                        let _ = session.close().await;
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            });
        })?;
    // The first time the desktop asks which display, so wait for the person to choose.
    let (node, fd, rect, mapping_id, token) = wait_for_stream(
        ready_rx,
        &stop_tx,
        stopping,
        request_failed,
        observe_control,
    )?;
    Ok((
        Stream::new(node, Some(fd), rect, mapping_id, Some(stop_tx)),
        token,
    ))
}

fn wait_for_stream<T>(
    ready_rx: flume::Receiver<Result<T>>,
    stop_tx: &flume::Sender<()>,
    stopping: &AtomicBool,
    request_failed: impl FnOnce(),
    mut observe_control: impl FnMut() -> Result<()>,
) -> Result<T> {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        if let Err(error) = observe_control() {
            let _ = stop_tx.try_send(());
            return Err(error);
        }
        if stopping.load(Ordering::Acquire) || std::time::Instant::now() >= deadline {
            let _ = stop_tx.try_send(());
            bail!("Screen sharing was cancelled or the dialog wasn't answered");
        }
        match ready_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => {
                request_failed();
                return Err(error);
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod consent_control_tests {
    use super::*;

    #[test]
    fn pending_consent_observes_control_before_waiting_for_portal_response() {
        let (_ready_tx, ready_rx) = flume::bounded::<Result<()>>(1);
        let (stop_tx, stop_rx) = flume::bounded(1);
        let stopping = AtomicBool::new(false);
        let result = wait_for_stream(
            ready_rx,
            &stop_tx,
            &stopping,
            || panic!("not a portal error"),
            || {
                stopping.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(stop_rx.try_recv(), Ok(()));
    }

    #[test]
    fn invalid_durable_control_closes_pending_consent_without_adopting_response() {
        let (ready_tx, ready_rx) = flume::bounded::<Result<()>>(1);
        ready_tx.send(Ok(())).unwrap();
        let (stop_tx, stop_rx) = flume::bounded(1);
        let result = wait_for_stream(
            ready_rx,
            &stop_tx,
            &AtomicBool::new(false),
            || panic!("not a portal error"),
            || bail!("invalid durable control binding"),
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid durable control binding"
        );
        assert_eq!(stop_rx.try_recv(), Ok(()));
    }
}
