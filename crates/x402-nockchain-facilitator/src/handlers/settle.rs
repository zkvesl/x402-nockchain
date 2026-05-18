//! `POST /settle` handler. Per `06-facilitator.md §6.3.2`.
//!
//! Envelope and signature checks mirror `/verify`; after those pass, the
//! settlement is routed through the configured
//! [`crate::ChainClient`]. Two paths:
//!
//! - [`StubChainClient`](crate::chain::StubChainClient) returns a
//!   deterministic synthetic tx_id with status `broadcast` — this is the
//!   Phase-2 regression path and is what the existing tests exercise.
//! - [`GrpcChainClient`](crate::chain::GrpcChainClient) submits to a real
//!   Nockchain node. In this commit it returns
//!   [`ChainError::TxConstructionUnavailable`] until the
//!   Authorization→RawTx translation is wired; callers get a 200 with a
//!   structured `error` payload so the failure is legible on the wire.

use std::time::Instant;

use axum::{extract::State, http::HeaderMap, Json};
use x402_types::facilitator::{
    FacilitatorError, SettleRequest, SettleResponse, TransactionStatus,
};

use crate::chain::ChainError;
use crate::handlers::{bazaar, verify};
use crate::observability::{label, metric};
use crate::AppState;

#[tracing::instrument(skip_all, fields(scheme = %req.requirements.scheme, network = %req.requirements.network))]
pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<SettleRequest>,
) -> (HeaderMap, Json<SettleResponse>) {
    let started = Instant::now();
    let mut headers = HeaderMap::new();

    // Use the replay-skipping variant: `/verify` already recorded
    // this nonce in the natural flow, and chain `tx_id` uniqueness
    // guards against settlement replays at the consensus layer.
    if let Err(e) = verify::verify_envelope_for_settle(&state, &req).await {
        let label_str = format!("envelope_rejected:{}", e.rejection_label());
        tracing::info!(code = %e.rejection_label(), message = %e, "settle rejected at envelope check");
        metrics::counter!(
            metric::REQUESTS_TOTAL,
            label::ENDPOINT => "/settle",
            label::OUTCOME => label_str.clone(),
        )
        .increment(1);
        metrics::histogram!(
            metric::SETTLE_LATENCY_SECONDS,
            label::OUTCOME => label_str,
        )
        .record(started.elapsed().as_secs_f64());

        return (
            headers,
            Json(SettleResponse {
                success: false,
                transaction: None,
                error: Some(FacilitatorError::from(e)),
            }),
        );
    }

    if let Some(outcome) = bazaar::process(&state, &req).await {
        if let Some(val) = bazaar::encode_header_value(&outcome) {
            headers.insert(bazaar::EXTENSION_RESPONSES_HEADER, val);
        }
    }

    // Per-scheme dispatch — `verify_envelope_for_settle` already
    // resolved the handler exists for this `(scheme, network)` pair,
    // so a `None` here would be a registry-mutation race, not a
    // legitimate request shape.
    let handler = match state
        .scheme_registry
        .handler_for(&req.requirements.scheme, &req.requirements.network)
    {
        Some(h) => h,
        None => {
            tracing::error!(
                scheme = %req.requirements.scheme,
                network = %req.requirements.network,
                "scheme registry race: handler resolved during verify but missing at settle",
            );
            return (
                headers,
                Json(SettleResponse {
                    success: false,
                    transaction: None,
                    error: Some(FacilitatorError {
                        code: "unknown_scheme".to_string(),
                        message: "scheme handler unavailable for settle".to_string(),
                    }),
                }),
            );
        }
    };

    let body = match handler.settle(state.chain.as_ref(), &req).await {
        Ok(result) => {
            tracing::info!(
                tx_id = %result.tx_id,
                status = %result.status.as_wire_str(),
                "settle accepted by chain client"
            );
            SettleResponse {
                success: true,
                transaction: Some(TransactionStatus {
                    tx_id: result.tx_id,
                    block_height: result.block_height,
                    status: result.status.as_wire_str().to_string(),
                }),
                error: None,
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "chain client returned error");
            SettleResponse {
                success: false,
                transaction: None,
                error: Some(chain_error_to_facilitator(&e)),
            }
        }
    };

    let outcome = if body.success {
        body.transaction
            .as_ref()
            .map(|t| t.status.as_str())
            .unwrap_or("ok")
    } else {
        body.error
            .as_ref()
            .map(|e| e.code.as_str())
            .unwrap_or("error")
    };
    metrics::counter!(
        metric::REQUESTS_TOTAL,
        label::ENDPOINT => "/settle",
        label::OUTCOME => outcome.to_string(),
    )
    .increment(1);
    metrics::histogram!(
        metric::SETTLE_LATENCY_SECONDS,
        label::OUTCOME => outcome.to_string(),
    )
    .record(started.elapsed().as_secs_f64());

    (headers, Json(body))
}

fn chain_error_to_facilitator(e: &ChainError) -> FacilitatorError {
    let (code, message) = match e {
        ChainError::Transport(m) => ("chain_transport", m.clone()),
        ChainError::Rejected(m) => ("chain_rejected", m.clone()),
        ChainError::TxConstructionUnavailable(m) => ("chain_unimplemented", m.clone()),
    };
    FacilitatorError {
        code: code.to_string(),
        message,
    }
}
