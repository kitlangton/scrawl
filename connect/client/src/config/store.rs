//! Config durability is implemented by the standalone native host crate.
pub use apparent_config_native::{
    ConfigPublicationUncertain, set_config_value_in, update_config_in,
    update_config_allowed_in,
};
#[cfg(any(test, feature = "test-support"))]
pub use apparent_config_native::{set_config_directory_sync_failure, set_config_write_hook};
