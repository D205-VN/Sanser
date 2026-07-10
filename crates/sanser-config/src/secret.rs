use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Eq, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl SecretString {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}
