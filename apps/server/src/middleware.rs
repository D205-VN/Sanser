use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, header::HeaderName},
    middleware::Next,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

pub static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub async fn request_id(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .filter(|value| valid_request_id(value))
        .cloned()
        .unwrap_or_else(|| {
            HeaderValue::from_str(&Uuid::new_v4().to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id"))
        });
    request
        .headers_mut()
        .insert(REQUEST_ID_HEADER.clone(), request_id.clone());
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(REQUEST_ID_HEADER.clone(), request_id);
    response
}

pub async fn rate_limit(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let key = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| "local".to_owned(), |address| address.0.ip().to_string());
    if !state.general_limiter.check(key).await {
        return AppError::RateLimited.into_response();
    }
    next.run(request).await
}

pub async fn timeout(State(state): State<AppState>, request: Request, next: Next) -> Response {
    match tokio::time::timeout(state.config.request_timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => AppError::Unavailable.into_response(),
    }
}

fn valid_request_id(value: &HeaderValue) -> bool {
    value.to_str().is_ok_and(|value| {
        (1..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}
