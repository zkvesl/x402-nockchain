//! Facilitator API request and response types.
//!
//! Shapes per PR #102 `specs/x402/06-facilitator.md §6.3.1` (`/verify`) and
//! `§6.3.2` (`/settle`), pinned under `docs/specs-snapshot/`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::payment::{PaymentPayload, PaymentRequirements};

// ---------------------------------------------------------------------------
// /verify
// ---------------------------------------------------------------------------

/// `POST /verify` request body. `payload` is the client's signed payload;
/// `requirements` is the `PaymentRequirements` entry the client selected
/// out of the server's 402 `accepts` list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub payload: PaymentPayload<Value>,
    pub requirements: PaymentRequirements,
}

/// `POST /verify` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<FacilitatorError>,
}

// ---------------------------------------------------------------------------
// /settle
// ---------------------------------------------------------------------------

/// `POST /settle` request body. Structurally identical to [`VerifyRequest`].
pub type SettleRequest = VerifyRequest;

/// `POST /settle` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction: Option<TransactionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<FacilitatorError>,
}

/// Settlement transaction status block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatus {
    pub tx_id: String,
    /// `None` until confirmed; the facilitator MAY return it as `null` on
    /// the wire per spec.
    #[serde(default)]
    pub block_height: Option<u64>,
    /// One of `"broadcast"`, `"confirmed"`, or an implementation-defined
    /// status string.
    pub status: String,
}

// ---------------------------------------------------------------------------
// Shared error shape
// ---------------------------------------------------------------------------

/// Common error payload returned by both `/verify` and `/settle`. Error
/// codes per `06-facilitator.md §6.6`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacilitatorError {
    pub code: String,
    pub message: String,
}
