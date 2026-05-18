//! Sign-In-With-Nockchain middleware.
//!
//! Wraps an axum handler with a require-SIWN gate: incoming requests must
//! carry a valid `SIGN-IN-WITH-X` header that verifies against the
//! configured domain. Verified identity is attached as a request
//! extension so downstream handlers can access it.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderName, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use x402_nockchain_crypto::{siwn, InMemoryReplayCache, VerifiedIdentity};

pub const SIWN_HEADER: HeaderName = HeaderName::from_static("sign-in-with-x");

/// Shared state the middleware needs across invocations. Clone cheaply.
#[derive(Clone)]
pub struct SiwnGate {
    pub expected_domain: Arc<String>,
    pub cache: Arc<InMemoryReplayCache>,
}

impl SiwnGate {
    pub fn new(expected_domain: impl Into<String>) -> Self {
        Self {
            expected_domain: Arc::new(expected_domain.into()),
            cache: Arc::new(InMemoryReplayCache::new()),
        }
    }
}

/// `axum::middleware::from_fn_with_state`-compatible middleware.
pub async fn require_siwn(
    axum::extract::State(gate): axum::extract::State<SiwnGate>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let header_value = match req.headers().get(&SIWN_HEADER) {
        Some(v) => v.clone(),
        None => return unauthorized("missing SIGN-IN-WITH-X header"),
    };
    let header_str = match header_value.to_str() {
        Ok(s) => s,
        Err(_) => return unauthorized("header is not valid UTF-8"),
    };

    match siwn::verify(header_str, &gate.expected_domain, gate.cache.as_ref(), Utc::now()) {
        Ok(identity) => {
            req.extensions_mut().insert::<VerifiedIdentity>(identity);
            next.run(req).await
        }
        Err(e) => unauthorized(&format!("SIWN rejected: {e}")),
    }
}

fn unauthorized(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, msg.to_string()).into_response()
}
