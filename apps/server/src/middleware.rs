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
    // A whole class may share one school address; the exam heartbeat and
    // screen have their own per-seat limits instead, and the supervisor's
    // screen polling a per-account one (routes::exams).
    if exam_heartbeat(request.uri().path()) || exam_screens(request.uri().path()) {
        return next.run(request).await;
    }
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

fn exam_heartbeat(path: &str) -> bool {
    path.strip_prefix("/api/v2/exam-members/")
        .and_then(|rest| {
            rest.strip_suffix("/heartbeat")
                .or_else(|| rest.strip_suffix("/screen"))
        })
        .is_some_and(|member| !member.is_empty() && !member.contains('/'))
}

fn exam_screens(path: &str) -> bool {
    path.strip_prefix("/api/v2/exams/")
        .and_then(|rest| rest.strip_suffix("/screens"))
        .is_some_and(|room| !room.is_empty() && !room.contains('/'))
}

fn valid_request_id(value: &HeaderValue) -> bool {
    value.to_str().is_ok_and(|value| {
        (1..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

#[cfg(test)]
mod tests {
    use super::{exam_heartbeat, exam_screens};

    #[test]
    fn only_exam_seat_and_screen_paths_skip_the_address_limit() {
        assert!(exam_heartbeat("/api/v2/exam-members/abc/heartbeat"));
        assert!(exam_heartbeat("/api/v2/exam-members/abc/screen"));
        assert!(exam_screens("/api/v2/exams/abc/screens"));
        assert!(!exam_screens("/api/v2/exams/abc/end"));
        assert!(!exam_screens("/api/v2/exams//screens"));
        assert!(!exam_screens("/api/v2/exams/a/b/screens"));
        assert!(!exam_heartbeat("/api/v2/exam-members/abc/unlock"));
    }
}
