//! `GET /discovery/resources` handler. Upstream-canonical query API shape:
//! `{items, pagination: {limit, offset, total}}`.
//!
//! Query params accepted:
//!
//! - `type` (upstream-canonical): kind filter, e.g. `mcp`, `http`.
//! - `limit`, `offset` (upstream-canonical): paginates the result.
//! - `network`, `scheme` (Nockchain-local extension; see ADR-0013):
//!   filter to resources whose `accepts[]` contains an entry matching
//!   the value. Proposed-upstream — not yet accepted in `coinbase/x402`.

use std::time::Instant;

use axum::{extract::{Query, State}, Json};
use serde::Deserialize;
use x402_types::bazaar::{
    DiscoveryResourcesResponse, ListDiscoveryResourcesParams, Pagination,
};

use crate::error::AppError;
use crate::observability::{label, metric};
use crate::AppState;

const X402_VERSION: u32 = 2;
const DEFAULT_LIMIT: u32 = 50;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    /// Filter to resources whose `accepts[]` contains an entry with the
    /// given network identifier. Nockchain-local extension; ADR-0013.
    #[serde(default)]
    pub network: Option<String>,
    /// Filter to resources whose `accepts[]` contains an entry with the
    /// given payment scheme. Nockchain-local extension; ADR-0013.
    #[serde(default)]
    pub scheme: Option<String>,
}

#[tracing::instrument(skip_all, fields(
    kind = ?params.kind,
    network = ?params.network,
    scheme = ?params.scheme,
))]
pub async fn list_resources(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<DiscoveryResourcesResponse>, AppError> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    let offset = params.offset.unwrap_or(0);

    let store_params = ListDiscoveryResourcesParams {
        kind: params.kind,
        limit: Some(limit),
        offset: Some(offset),
        network: params.network,
        scheme: params.scheme,
    };

    let started = Instant::now();
    let result = state.catalog.list(&store_params).await;
    let latency_secs = started.elapsed().as_secs_f64();

    let outcome = if result.is_ok() { "ok" } else { "error" };
    metrics::counter!(
        metric::REQUESTS_TOTAL,
        label::ENDPOINT => "/discovery/resources",
        label::OUTCOME => outcome,
    )
    .increment(1);
    metrics::histogram!(
        metric::CATALOG_LIST_LATENCY_SECONDS,
        label::OUTCOME => outcome,
    )
    .record(latency_secs);

    let (items, total) = result.map_err(AppError::from)?;

    Ok(Json(DiscoveryResourcesResponse {
        x402_version: X402_VERSION,
        items,
        pagination: Pagination { limit, offset, total },
    }))
}
