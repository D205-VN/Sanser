//! Authentication building blocks with deliberately redacted secret types.

mod password;
mod token;
mod totp;

pub use password::{
    PasswordError, PasswordHashString, PasswordHasherService, PasswordPolicy, PasswordPolicyError,
};
pub use token::{
    IssuedToken, SecretToken, TokenDigest, TokenError, TokenKey, TokenKind, TokenRecord,
    TokenService,
};
pub use totp::{
    SecretBox, TOTP_DIGITS, TOTP_PERIOD_SECONDS, TotpError, TotpSecret, generate_recovery_code,
    normalize_recovery_code, normalize_totp_code,
};
