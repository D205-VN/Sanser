//! Central configuration parsing and validation.
//!
//! Values are parsed from a supplied iterator so tests do not need to mutate
//! process-global environment state. Production data uses `PostgreSQL` on Neon.

mod secret;
mod validate;

pub use secret::SecretString;
pub use validate::{
    AppConfig, AuthConfig, ConfigError, DatabaseConfig, NetworkConfig, ServerConfig, TurnConfig,
};
