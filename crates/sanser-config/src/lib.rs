//! Central configuration parsing and validation.
//!
//! Values are parsed from a supplied iterator so tests and embedded local mode
//! do not need to mutate process-global environment state.

mod secret;
mod validate;

pub use secret::SecretString;
pub use validate::{
    AppConfig, AuthConfig, ConfigError, DatabaseConfig, NetworkConfig, ServerConfig, TurnConfig,
};
