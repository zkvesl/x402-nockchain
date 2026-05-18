//! `POST /verify` handler. Per `06-facilitator.md §6.3.1` + `§6.6`.
//!
//! Pre-dispatch (scheme-independent):
//! - `x402Version == 2`.
//! - `payload.scheme == requirements.scheme`,
//!   `payload.network == requirements.network`.
//! - R1.1 `§6.6` time-window check (with skew tolerance).
//! - R1.1 `§6.6` Authorization-nonce replay check (Consume on
//!   `/verify`, Skip on `/settle` — the natural follow-up flow records
//!   the nonce on `/verify`).
//!
//! Per-scheme dispatch (R1.2): the configured
//! [`crate::SchemeHandlerRegistry`] resolves the `(scheme, network)`
//! pair to a [`crate::SchemeHandler`]. The handler runs payload decode,
//! `to == payTo`, signature verification, max-amount cap, and asset
//! match. Unknown `(scheme, network)` pairs reject with
//! [`VerifyError::UnknownScheme`] — pre-R1.2 silently `Ok`'d them
//! (ADR-0020 captures the behavior change).

use std::time::Instant;

use axum::{extract::State, http::HeaderMap, Json};
use x402_nockchain_crypto::replay_domains;
use x402_types::facilitator::{FacilitatorError, VerifyRequest, VerifyResponse};
use x402_types::payment::Authorization;

use crate::error::VerifyError;
use crate::handlers::bazaar;
use crate::observability::{label, metric};
use crate::AppState;

#[tracing::instrument(skip_all, fields(scheme = %req.requirements.scheme, network = %req.requirements.network))]
pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<VerifyRequest>,
) -> (HeaderMap, Json<VerifyResponse>) {
    let started = Instant::now();
    let mut headers = HeaderMap::new();

    let outcome_label;
    let body = match verify_envelope(&state, &req).await {
        Ok(()) => {
            tracing::debug!("envelope + signature verified");
            if let Some(outcome) = bazaar::process(&state, &req).await {
                if let Some(val) = bazaar::encode_header_value(&outcome) {
                    headers.insert(bazaar::EXTENSION_RESPONSES_HEADER, val);
                }
            }
            outcome_label = "ok".to_string();
            VerifyResponse { valid: true, error: None }
        }
        Err(e) => {
            tracing::info!(code = %e.rejection_label(), message = %e, "verify rejected");
            outcome_label = format!("rejected:{}", e.rejection_label());
            VerifyResponse {
                valid: false,
                error: Some(FacilitatorError::from(e)),
            }
        }
    };

    metrics::counter!(
        metric::REQUESTS_TOTAL,
        label::ENDPOINT => "/verify",
        label::OUTCOME => outcome_label.clone(),
    )
    .increment(1);
    metrics::histogram!(
        metric::VERIFY_LATENCY_SECONDS,
        label::OUTCOME => outcome_label,
    )
    .record(started.elapsed().as_secs_f64());

    (headers, Json(body))
}

/// Apply envelope, time-window, replay, and per-scheme handler checks.
/// Returns the first failure as a typed [`VerifyError`] (which converts
/// to the wire [`FacilitatorError`] via `From`), or `Ok(())`.
///
/// Order matters: cheap envelope checks first, then time-window
/// (so a clock-stale message doesn't pollute the cache), then replay,
/// then the handler dispatch (signature, recipient, max-amount, asset).
///
/// Exposed `pub` so direct callers (and the integration-test crate)
/// can drive the function without an HTTP round trip and pattern-match
/// on the typed [`VerifyError`].
pub async fn verify_envelope(
    state: &AppState,
    req: &VerifyRequest,
) -> Result<(), VerifyError> {
    verify_envelope_inner(state, req, ReplayMode::Consume).await
}

/// Same checks as [`verify_envelope`] minus the nonce-replay check.
/// Used by `/settle` per `06-facilitator.md §6.3.2`: the natural
/// verify-then-settle flow records the nonce on `/verify`'s consume,
/// and `/settle` should accept the same envelope on the follow-up
/// without re-rejecting on the cache hit. Replay protection on
/// settlement comes from the chain's `tx_id` uniqueness; the
/// facilitator continues to enforce signature, time-window, cap, and
/// asset on `/settle` for defense in depth.
pub async fn verify_envelope_for_settle(
    state: &AppState,
    req: &VerifyRequest,
) -> Result<(), VerifyError> {
    verify_envelope_inner(state, req, ReplayMode::Skip).await
}

#[derive(Debug, Clone, Copy)]
enum ReplayMode {
    Consume,
    Skip,
}

async fn verify_envelope_inner(
    state: &AppState,
    req: &VerifyRequest,
    replay_mode: ReplayMode,
) -> Result<(), VerifyError> {
    if req.payload.x402_version != 2 {
        return Err(spec("invalid_version", "x402Version MUST be 2"));
    }
    if req.payload.scheme != req.requirements.scheme {
        return Err(spec(
            "invalid_scheme",
            "payload.scheme does not match requirements.scheme",
        ));
    }
    if req.payload.network != req.requirements.network {
        return Err(spec(
            "invalid_network",
            "payload.network does not match requirements.network",
        ));
    }

    // ---- Per-scheme dispatch ----
    let handler = state
        .scheme_registry
        .handler_for(&req.requirements.scheme, &req.requirements.network)
        .ok_or_else(|| VerifyError::UnknownScheme {
            scheme: req.requirements.scheme.clone(),
            network: req.requirements.network.clone(),
        })?;

    // The handler exposes the universal `Authorization` view so the
    // pre-dispatch path can run the scheme-independent §6.6 checks
    // (time window, replay) without hardcoding a particular payload
    // shape.
    let authorization = handler.extract_authorization(&req.payload)?;

    check_time_window(state, &authorization)?;
    if matches!(replay_mode, ReplayMode::Consume) {
        check_replay(state, &authorization)?;
    }

    handler.verify(&req.payload, &req.requirements).await?;

    Ok(())
}

/// Compose a backwards-compatible [`VerifyError::SpecCoded`].
fn spec(code: &str, message: impl Into<String>) -> VerifyError {
    VerifyError::SpecCoded {
        code: code.to_string(),
        message: message.into(),
    }
}

/// Reject when `valid_after > now + skew` or `valid_before < now -
/// skew`. The skew tolerance is configured on [`AppState`] and applied
/// symmetrically — we add it on the upper edge (a client whose clock
/// runs slightly fast still authorizes) and subtract it on the lower
/// edge (an authorization that just expired by a few seconds is still
/// honored). Both edges are u64-safe via saturating arithmetic.
fn check_time_window(state: &AppState, auth: &Authorization) -> Result<(), VerifyError> {
    let now = state.clock.now();
    let skew = state.clock_skew_tolerance.as_secs();
    let upper_edge = now.saturating_add(skew);
    let lower_edge = now.saturating_sub(skew);
    if auth.valid_after > upper_edge {
        return Err(VerifyError::OutsideTimeWindow {
            now,
            valid_after: auth.valid_after,
            valid_before: auth.valid_before,
        });
    }
    if auth.valid_before < lower_edge {
        return Err(VerifyError::OutsideTimeWindow {
            now,
            valid_after: auth.valid_after,
            valid_before: auth.valid_before,
        });
    }
    Ok(())
}

/// Reject when `Authorization.nonce` has been observed within the
/// configured replay-cache TTL. The cache is shared with the SIWN
/// middleware via [`x402_nockchain_crypto::ReplayCache`], so the nonce
/// is domain-prefixed (`x402-auth:`) to prevent collision with SIWN's
/// `siwn:` namespace.
fn check_replay(state: &AppState, auth: &Authorization) -> Result<(), VerifyError> {
    let key = x402_nockchain_crypto::prefixed_replay_key(
        replay_domains::AUTHORIZATION,
        auth.nonce.as_bytes(),
    );
    if state.replay_cache.seen(&key, state.replay_ttl) {
        return Err(VerifyError::NonceReplayed {
            nonce: auth.nonce.clone(),
        });
    }
    Ok(())
}
