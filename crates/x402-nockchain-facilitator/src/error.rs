//! Axum-compatible error type plus a typed `VerifyError` for the
//! verifier-policy hardening landed in R1.1.
//!
//! Most facilitator errors are spec-defined `FacilitatorError` payloads
//! returned inside a 200 `VerifyResponse` / `SettleResponse`. Only
//! infrastructure-level failures (bad JSON, DB errors) surface as HTTP
//! error status codes.
//!
//! `VerifyError` is the typed surface used by `verify_envelope` and its
//! tests; `From<VerifyError> for FacilitatorError` maps each variant to
//! the spec's `§6.6` error code so the wire format is unchanged.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use x402_types::facilitator::FacilitatorError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("internal error: {0}")]
    Internal(String),
}

/// Typed verifier outcomes for `verify_envelope`. Implements
/// [`From<VerifyError> for FacilitatorError`] so the on-wire shape is
/// the existing `{ code, message }` pair; tests match against these
/// variants for clean `assert!(matches!(...))` patterns.
///
/// Each variant carries the pieces the rejection-code Prom counter
/// labels need (`outcome="rejected:<variant>"`) plus a human-readable
/// message. New variants since R1.1: `OutsideTimeWindow`,
/// `NonceReplayed`, `OverMaxAmount`, `AssetMismatch`. Pre-R1.1
/// rejection codes (`invalid_version`, `invalid_scheme`, etc.) keep
/// their stringly-typed shape and surface as
/// [`VerifyError::SpecCoded`] for backwards compatibility — older
/// callers that constructed `FacilitatorError` directly continue to
/// work.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum VerifyError {
    /// `Authorization.valid_after` is in the future, or
    /// `Authorization.valid_before` is in the past, beyond the
    /// configured clock-skew tolerance.
    #[error("authorization outside time window: now={now} valid_after={valid_after} valid_before={valid_before}")]
    OutsideTimeWindow {
        now: u64,
        valid_after: u64,
        valid_before: u64,
    },
    /// `Authorization.nonce` was observed within the replay-cache TTL.
    #[error("authorization nonce already seen: {nonce}")]
    NonceReplayed { nonce: String },
    /// `Authorization.value` exceeds `PaymentRequirements.max_amount_required`
    /// for the `(exact, *)` scheme. The `upto` scheme has different
    /// semantics and is gated to R1.2.
    #[error("authorization value {value} exceeds max_amount_required {max_amount_required}")]
    OverMaxAmount {
        value: String,
        max_amount_required: String,
    },
    /// `Authorization.asset` (Nockchain wallet payloads do not carry
    /// `auth.asset` directly; the implicit asset for `(exact,
    /// nockchain:*)` is `"NOCK"`) does not match `requirements.asset`.
    #[error("asset mismatch: requirements.asset={expected}, payload asset={actual}")]
    AssetMismatch { expected: String, actual: String },
    /// No registered scheme handler matched `(scheme, network)`. Pre-R1.2
    /// the verifier silently `Ok`'d this case; per ADR-0020 it now
    /// rejects.
    #[error("unknown scheme/network pair: scheme={scheme} network={network}")]
    UnknownScheme { scheme: String, network: String },
    /// One of the pre-R1.1 spec-coded rejections (invalid version,
    /// scheme mismatch, payload decode, recipient mismatch, signature
    /// invalid). Carries the §6.6 string code verbatim.
    #[error("{code}: {message}")]
    SpecCoded { code: String, message: String },
}

impl VerifyError {
    /// Stable string label suitable for the `outcome="rejected:<label>"`
    /// Prometheus counter. Pre-R1.1 spec-coded variants reuse the §6.6
    /// code so existing dashboards keep their bucket names.
    pub fn rejection_label(&self) -> &str {
        match self {
            VerifyError::OutsideTimeWindow { .. } => "outside_time_window",
            VerifyError::NonceReplayed { .. } => "nonce_replayed",
            VerifyError::OverMaxAmount { .. } => "over_max_amount",
            VerifyError::AssetMismatch { .. } => "asset_mismatch",
            VerifyError::UnknownScheme { .. } => "unknown_scheme",
            VerifyError::SpecCoded { code, .. } => code.as_str(),
        }
    }
}

impl From<VerifyError> for FacilitatorError {
    fn from(e: VerifyError) -> Self {
        match e {
            VerifyError::OutsideTimeWindow {
                now,
                valid_after,
                valid_before,
            } => FacilitatorError {
                code: "outside_time_window".to_string(),
                message: format!(
                    "authorization outside time window: now={now} valid_after={valid_after} valid_before={valid_before}",
                ),
            },
            VerifyError::NonceReplayed { nonce } => FacilitatorError {
                code: "nonce_replayed".to_string(),
                message: format!("authorization nonce already seen: {nonce}"),
            },
            VerifyError::OverMaxAmount {
                value,
                max_amount_required,
            } => FacilitatorError {
                code: "over_max_amount".to_string(),
                message: format!(
                    "authorization value {value} exceeds max_amount_required {max_amount_required}",
                ),
            },
            VerifyError::AssetMismatch { expected, actual } => FacilitatorError {
                code: "asset_mismatch".to_string(),
                message: format!(
                    "asset mismatch: requirements.asset={expected}, payload asset={actual}",
                ),
            },
            VerifyError::UnknownScheme { scheme, network } => FacilitatorError {
                code: "unknown_scheme".to_string(),
                message: format!(
                    "unknown scheme/network pair: scheme={scheme} network={network}",
                ),
            },
            VerifyError::SpecCoded { code, message } => FacilitatorError { code, message },
        }
    }
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let message = match &self {
            Self::Internal(m) => m.clone(),
        };
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": { "code": "facilitator_internal_error", "message": message } })),
        )
            .into_response()
    }
}
