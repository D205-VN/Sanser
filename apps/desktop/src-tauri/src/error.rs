use serde::ser::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("required component is unavailable: {0}")]
    Unavailable(String),
    #[error("secure storage operation failed")]
    SecureStorage,
    #[error("local storage operation failed: {0}")]
    Storage(String),
    #[error("native process operation failed: {0}")]
    Process(String),
}

impl Serialize for DesktopError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<std::io::Error> for DesktopError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}
