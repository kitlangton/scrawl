#[cfg(feature = "provider-wire")]
mod wire;

#[cfg(feature = "provider-wire")]
pub use wire::{Action, DeadlineStream, clock_ms, connect_until, same_user, socket_path, valid_id};

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
pub use client::{command, command_at, execute, execute_with, new_id};
