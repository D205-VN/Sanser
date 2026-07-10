pub mod auth;
pub mod config;
pub mod db;
pub mod error;
mod events;
mod middleware;
pub mod models;
pub mod routes;
pub mod state;
mod time;
pub mod websocket;

use std::{net::SocketAddr, time::Duration};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::{Method, header},
    middleware as axum_middleware,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{
    config::Config,
    error::{AppError, AppError::NotFound},
    state::AppState,
};

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("database initialization failed")]
    Database(#[source] AppError),
    #[error("failed to bind server socket: {0}")]
    Bind(#[source] std::io::Error),
    #[error("server terminated with an I/O error: {0}")]
    Serve(#[source] std::io::Error),
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(state.config.allowed_origins.clone())
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            middleware::REQUEST_ID_HEADER.clone(),
        ])
        .expose_headers([middleware::REQUEST_ID_HEADER.clone()])
        .max_age(Duration::from_secs(600));

    Router::new()
        .route("/api/v2/auth/register", post(routes::auth::register))
        .route("/api/v2/auth/login", post(routes::auth::login))
        .route("/api/v2/auth/logout", post(routes::auth::logout))
        .route("/api/v2/auth/refresh", post(routes::auth::refresh))
        .route("/api/v2/account", get(routes::account::get_account))
        .route(
            "/api/v2/account/password",
            post(routes::account::change_password),
        )
        .route(
            "/api/v2/account/sessions",
            get(routes::account::list_login_sessions),
        )
        .route(
            "/api/v2/account/sessions/{id}",
            delete(routes::account::revoke_login_session),
        )
        .route("/api/v2/devices", get(routes::devices::list))
        .route("/api/v2/devices/register", post(routes::devices::register))
        .route(
            "/api/v2/devices/heartbeat",
            post(routes::devices::heartbeat),
        )
        .route(
            "/api/v2/devices/{id}",
            patch(routes::devices::update).delete(routes::devices::remove),
        )
        .route("/api/v2/sessions", post(routes::sessions::create))
        .route("/api/v2/sessions/{id}", get(routes::sessions::get))
        .route(
            "/api/v2/sessions/{id}/accept",
            post(routes::sessions::accept),
        )
        .route(
            "/api/v2/sessions/{id}/reject",
            post(routes::sessions::reject),
        )
        .route(
            "/api/v2/sessions/{id}/disconnect",
            post(routes::sessions::disconnect),
        )
        .route("/api/v2/network/ice", get(routes::network::ice))
        .route("/api/v2/health", get(routes::system::health))
        .route("/api/v2/readiness", get(routes::system::readiness))
        .route("/api/v2/events", get(websocket::events_socket))
        .route("/api/v2/signaling", get(websocket::signaling_socket))
        .fallback(api_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state.clone())
        .layer(DefaultBodyLimit::max(state.config.body_limit_bytes))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                        request_id = request
                            .headers()
                            .get(&middleware::REQUEST_ID_HEADER)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("unknown")
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::timeout,
        ))
        .layer(axum_middleware::from_fn_with_state(
            state,
            middleware::rate_limit,
        ))
        .layer(cors)
        .layer(axum_middleware::from_fn(middleware::request_id))
}

pub async fn serve(config: Config) -> Result<(), ServerError> {
    let address = SocketAddr::new(config.host, config.port);
    let pool = db::connect(&config).await.map_err(ServerError::Database)?;
    let state = AppState::new(config, pool.clone());
    let cancellation = CancellationToken::new();
    let cleanup_task = spawn_cleanup(state.clone(), cancellation.clone());
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(ServerError::Bind)?;
    tracing::info!(address = %listener.local_addr().unwrap_or(address), "Sanser API server listening");

    let result = axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(cancellation.clone()))
    .await
    .map_err(ServerError::Serve);
    cancellation.cancel();
    let _ = cleanup_task.await;
    pool.close().await;
    result
}

fn spawn_cleanup(state: AppState, cancellation: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => break,
                _ = interval.tick() => {
                    let now = time::now_unix();
                    let offline_before = now.saturating_sub(
                        i64::try_from(state.config.device_offline_after.as_secs()).unwrap_or(i64::MAX)
                    );
                    let pending_before = now.saturating_sub(
                        i64::try_from(state.config.pending_session_ttl.as_secs()).unwrap_or(i64::MAX)
                    );
                    db::cleanup(&state.pool, now, offline_before, pending_before).await;
                }
            }
        }
    })
}

async fn shutdown_signal(cancellation: CancellationToken) {
    #[cfg(unix)]
    {
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                }
                Err(error) => {
                    tracing::warn!(error = %error, "failed to install SIGTERM handler");
                    std::future::pending::<()>().await;
                }
            }
        };
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::warn!(error = %error, "failed to listen for Ctrl-C");
                }
            },
            _ = terminate => {},
            _ = cancellation.cancelled() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::warn!(error = %error, "failed to listen for Ctrl-C");
            }
        },
        _ = cancellation.cancelled() => {},
    }
    cancellation.cancel();
}

async fn api_not_found() -> AppError {
    NotFound
}

async fn method_not_allowed() -> impl IntoResponse {
    (
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        axum::Json(serde_json::json!({
            "error": {"code": "method_not_allowed", "message": "method not allowed"}
        })),
    )
}
