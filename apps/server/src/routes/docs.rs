//! Human-readable API reference at `/api/docs`, backed by `/api/openapi.json`.
//! Both are static and unauthenticated; they describe routes, not data.
use axum::{
    http::{HeaderValue, header},
    response::{Html, IntoResponse},
};

const OPENAPI: &str = include_str!("../../openapi.json");
const PAGE: &str = include_str!("../../assets/docs.html");

/// The page is self-contained (no third-party scripts) and may only call this
/// origin, so a pasted access token cannot be sent anywhere else by the page.
const PAGE_POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub async fn openapi() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            ),
        ],
        OPENAPI,
    )
}

pub async fn page() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(PAGE_POLICY),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        Html(PAGE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The reference must list exactly the `/api/v2` routes the router serves.
    #[test]
    fn openapi_lists_every_api_route_and_nothing_else() {
        let spec: serde_json::Value = serde_json::from_str(OPENAPI).expect("valid OpenAPI JSON");
        let documented: BTreeSet<String> = spec["paths"]
            .as_object()
            .expect("paths")
            .keys()
            .cloned()
            .collect();
        let router = include_str!("../lib.rs");
        let served: BTreeSet<String> = router
            .split('"')
            .filter(|part| part.starts_with("/api/v2/"))
            .map(str::to_owned)
            .collect();
        assert!(served.len() > 30, "router routes were not found");
        assert_eq!(documented, served);
    }

    #[test]
    fn every_operation_is_tagged_and_references_existing_schemas() {
        let spec: serde_json::Value = serde_json::from_str(OPENAPI).expect("valid OpenAPI JSON");
        let schemas = spec["components"]["schemas"].as_object().expect("schemas");
        for (path, methods) in spec["paths"].as_object().expect("paths") {
            for (method, operation) in methods.as_object().expect("methods") {
                assert!(
                    operation["tags"]
                        .as_array()
                        .is_some_and(|tags| tags.len() == 1),
                    "{method} {path} needs one tag"
                );
            }
        }
        for reference in OPENAPI.split("#/components/schemas/").skip(1) {
            let name = reference.split('"').next().unwrap_or_default();
            assert!(schemas.contains_key(name), "missing schema {name}");
        }
    }

    #[test]
    fn page_is_self_contained() {
        assert!(!PAGE.contains("http://") && !PAGE.contains("https://"));
        assert!(PAGE.contains("/api/openapi.json"));
    }
}
