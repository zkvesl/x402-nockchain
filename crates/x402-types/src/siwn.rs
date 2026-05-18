//! Sign-In-With-Nockchain (SIWN) types — CAIP-122 based authentication.
//!
//! Only the types-only portion lands here in Phase 1. Sign/verify wiring
//! (`"siwn-v1"` Tip5 domain separator, Schnorr over the canonical message
//! body) is scheduled for Phase 3.
//!
//! Shape tracks PR #102 `specs/x402/11-extensions.md §11.2`, pinned under
//! `docs/specs-snapshot/`.

use serde::{Deserialize, Serialize};

/// The block that sits under `PaymentRequirements.extra.siwn`. The server
/// emits it alongside a 402 response to let the client prove a prior
/// payment without re-paying.
///
/// The short form in the spec's §11.2.3 example carries only `domain`,
/// `nonce`, `issuedAt`, and `expirationTime`; `uri` and `chainId` are
/// signed-message components the client may derive from the outer
/// `resource` / `network` fields. Both shapes must round-trip, so `uri`
/// and `chainId` are optional here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SiwnExtra {
    /// Domain the server is asserting (typically the resource's hostname).
    pub domain: String,
    /// Server-generated random nonce; client echoes it in the signed
    /// message body.
    pub nonce: String,
    /// ISO-8601 timestamp at which the challenge was issued.
    pub issued_at: String,
    /// ISO-8601 timestamp after which the challenge is no longer valid.
    pub expiration_time: String,
    /// Absolute URI of the resource being authenticated against. Often
    /// derivable from `PaymentRequirements.resource`; optional here so the
    /// short spec example round-trips without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// CAIP-2 chain identifier (e.g. `"nockchain:mainnet"`). Often
    /// derivable from `PaymentRequirements.network`; optional here for
    /// the same reason as `uri`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
}
