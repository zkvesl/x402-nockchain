//! Operator-readable observability surface — JSON-formatted `tracing`
//! logs and a Prometheus `/metrics` endpoint.
//!
//! Tracing uses the layered subscriber pattern from `tracing-subscriber`
//! so a downstream operator can plug in any additional layer (OTLP,
//! Loki, etc.) without forking this crate. The default
//! [`init_tracing`] call sets up an env-filtered JSON formatter; if the
//! caller has already initialised a global subscriber, [`init_tracing`]
//! returns `Ok(())` rather than failing.
//!
//! Metric label cardinality is kept low on purpose. Counters tag by
//! endpoint + outcome bucket only; per-resource / per-pkh labels would
//! explode cardinality and kill the metrics backend.
//!
//! See ADR-0016 for the design rationale.

use std::sync::OnceLock;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// ---------------------------------------------------------------------------
// Tracing
// ---------------------------------------------------------------------------

/// Initialise the global `tracing` subscriber with a JSON-formatted
/// stdout layer and an env-filter layer (`RUST_LOG` overrides, default
/// `info`).
///
/// Idempotent: if a global subscriber is already set (e.g. by an
/// integration-test harness), this is a no-op.
pub fn init_tracing() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let layer = fmt::layer().json().with_target(true).with_current_span(true);
    let registry = tracing_subscriber::registry().with(filter).with(layer);
    let _ = registry.try_init();
    Ok(())
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Process-global Prometheus handle. Initialised once on first call to
/// [`install_metrics_recorder`] and reused thereafter.
static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install a Prometheus metrics recorder as the global `metrics`
/// recorder, returning a handle that can render the current snapshot.
///
/// Idempotent — repeated calls return the cached handle.
pub fn install_metrics_recorder() -> Result<MetricsHandle> {
    if let Some(handle) = METRICS_HANDLE.get() {
        return Ok(MetricsHandle(handle.clone()));
    }
    let builder = PrometheusBuilder::new();
    let handle = builder
        .install_recorder()
        .context("install Prometheus recorder")?;
    let _ = METRICS_HANDLE.set(handle.clone());
    Ok(MetricsHandle(handle))
}

/// Cheap-to-clone wrapper around the Prometheus exporter handle. Carries
/// the handle to the `/metrics` axum router built by [`metrics_router`].
#[derive(Clone)]
pub struct MetricsHandle(PrometheusHandle);

impl MetricsHandle {
    /// Render the current snapshot in Prometheus text-exposition format.
    pub fn render(&self) -> String {
        self.0.render()
    }
}

/// Build a tiny axum `Router` exposing `GET /metrics`. Usually mounted
/// alongside the facilitator's main router via `Router::merge`.
pub fn metrics_router(handle: MetricsHandle) -> Router {
    Router::new()
        .route("/metrics", get(render_metrics))
        .with_state(handle)
}

async fn render_metrics(State(handle): State<MetricsHandle>) -> Response {
    let body = handle.render();
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Metric name + label conventions
// ---------------------------------------------------------------------------
//
// Single source of truth for metric names; handlers reference these
// constants rather than stringly-typed names so a misspelling is a
// compile error.

pub mod metric {
    pub const REQUESTS_TOTAL: &str = "x402_requests_total";
    pub const VERIFY_LATENCY_SECONDS: &str = "x402_verify_latency_seconds";
    pub const SETTLE_LATENCY_SECONDS: &str = "x402_settle_latency_seconds";
    pub const CATALOG_LIST_LATENCY_SECONDS: &str = "x402_catalog_list_latency_seconds";
    pub const CATALOG_UPSERTS_TOTAL: &str = "x402_catalog_upserts_total";
}

pub mod label {
    pub const ENDPOINT: &str = "endpoint";
    pub const OUTCOME: &str = "outcome";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metrics_router_renders_text() {
        let handle = install_metrics_recorder().expect("install");
        // Bump a counter so the snapshot has at least one line.
        metrics::counter!(metric::REQUESTS_TOTAL, label::ENDPOINT => "/verify",
                          label::OUTCOME => "ok")
            .increment(1);

        let body = handle.render();
        assert!(
            body.contains(metric::REQUESTS_TOTAL),
            "rendered snapshot missing requests counter:\n{body}"
        );
    }
}
