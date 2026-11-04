use anyhow::{Result, ensure};
use apparent_capture::{self as agent_capture, Action};
use clap::{Parser, Subcommand};
use std::time::Duration;
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: CaptureCommand,
}
#[derive(Subcommand)]
enum CaptureCommand {
    /// Read the running GUI's phase and opaque capture identity as JSON.
    State,
    /// Start the normal GUI screen capture without toggling or launching an app.
    Start {
        /// Wait until a real screen frame arrives (portal consent may be needed).
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds. Timeout leaves an admitted capture intact; inspect its ID.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
        /// Retry a known start request without starting a second capture (32 hex characters).
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Finish only a matching, already-active capture.
    Stop {
        capture_id: String,
        /// Wait for this capture's saved bundle, not merely stop acceptance.
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds. Retry or inspect the same ID after timeout.
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
    },
    /// Discard only a matching pending or active capture.
    Cancel { capture_id: String },
    /// Read this capture's result, retained only for the running GUI's lifetime.
    Inspect { capture_id: String },
}

fn main() -> Result<()> {
    let (action, id, wait, timeout) = match Cli::parse().command {
        CaptureCommand::State => (None, None, false, 4),
        CaptureCommand::Start {
            wait,
            timeout,
            request_id,
        } => (
            Some(Action::Start),
            Some(match request_id {
                Some(id) => id,
                None => agent_capture::new_id()?,
            }),
            wait,
            timeout,
        ),
        CaptureCommand::Stop {
            capture_id,
            wait,
            timeout,
        } => (Some(Action::Stop), Some(capture_id), wait, timeout),
        CaptureCommand::Cancel { capture_id } => (Some(Action::Cancel), Some(capture_id), false, 4),
        CaptureCommand::Inspect { capture_id } => {
            (Some(Action::Inspect), Some(capture_id), false, 4)
        }
    };
    let value = agent_capture::execute(
        action.zip(id.as_deref()),
        wait,
        Duration::from_secs(timeout),
    );
    println!("{value}");
    ensure!(!value["error"].is_string(), "GUI capture control: {value}");
    ensure!(
        !matches!(
            value["outcome"].as_str(),
            Some("wrong_capture" | "not_ready")
        ),
        "GUI capture control: {value}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_surface_and_timeout_bounds() {
        for args in [
            vec!["capture", "record"],
            vec!["capture", "start", "--timeout", "0"],
            vec!["capture", "stop", "id", "--timeout", "3601"],
            vec!["capture", "start", "--window", "42"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
        assert!(Cli::try_parse_from(["capture", "state"]).is_ok());
        assert!(Cli::try_parse_from(["capture", "start", "--wait", "--timeout", "3600"]).is_ok());
    }
}
