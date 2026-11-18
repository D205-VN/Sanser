use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("authentication is required")]
    Unauthorized,
    #[error("you are not allowed to perform this action")]
    Forbidden,
    #[error("the requested resource was not found")]
    NotFound,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("too many requests")]
    RateLimited,
    #[error("request payload is too large")]
    PayloadTooLarge,
    #[error("service is temporarily unavailable")]
    Unavailable,
    #[error("internal server error")]
    Internal,
    /// The password was correct and the account uses two-step verification.
    /// Older clients show the message; newer ones read the challenge.
    #[error(
        "Two-step verification is turned on for this account. Update Sanser to the latest version to sign in."
    )]
    TwoFactorRequired {
        challenge_token: String,
        expires_at: i64,
    },
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorDetail {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    challenge_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    challenge_expires_at: Option<i64>,
}

impl AppError {
    pub fn from_db(error: sqlx::Error) -> Self {
        tracing::error!(error = %error, "database operation failed");
        Self::Internal
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized | Self::TwoFactorRequired { .. } => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Validation(_) => "validation_error",
            Self::Conflict(_) => "conflict",
            Self::RateLimited => "rate_limited",
            Self::PayloadTooLarge => "payload_too_large",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal_error",
            Self::TwoFactorRequired { .. } => "two_factor_required",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let message = self.to_string();
        let code = self.code();
        let (challenge_token, challenge_expires_at) = match self {
            Self::TwoFactorRequired {
                challenge_token,
                expires_at,
            } => (Some(challenge_token), Some(expires_at)),
            _ => (None, None),
        };
        (
            status,
            Json(ErrorResponse {
                error: ErrorDetail {
                    code,
                    message,
                    challenge_token,
                    challenge_expires_at,
                },
            }),
        )
            .into_response()
    }
}

pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(JsonRejection::BytesRejection(_)) => Err(AppError::PayloadTooLarge),
            Err(error) => Err(AppError::Validation(format!(
                "invalid JSON payload: {}",
                error.body_text()
            ))),
        }
    }
}
