//! Payment-protocol types: [`PaymentRequirements`], [`PaymentPayload`],
//! [`Authorization`], [`SchnorrSignatureJson`], [`ExtensionResponsesHeader`].
//!
//! Shapes track PR #102 `specs/x402/04-payment-requirements.md` and
//! `05-payment-payload.md`, pinned under `docs/specs-snapshot/`. See
//! [`crate::nockchain`] for the Nockchain-specific `payload` variant
//! (`ExactNockchainPayload`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// PaymentRequirements — one entry of the 402 response's `accepts` array
// ---------------------------------------------------------------------------

/// A single payment option offered by the resource server, describing *how*
/// the client may pay to access the resource.
///
/// Shape matches PR #102 `specs/x402/04-payment-requirements.md`. The 402
/// response envelope (not modeled in this crate) wraps `accepts:
/// Vec<PaymentRequirements>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequirements {
    /// Payment scheme identifier (e.g. `"exact"`, `"upto"`).
    pub scheme: String,
    /// CAIP-2-style network identifier (e.g. `"nockchain:mainnet"`).
    pub network: String,
    /// Maximum amount the client may be charged for this resource, in the
    /// asset's smallest unit (for Nockchain: nicks). Serialized as a decimal
    /// string to preserve u128 precision.
    pub max_amount_required: String,
    /// Absolute URL of the resource being paid for.
    pub resource: String,
    /// Asset identifier (network-specific encoding; Nockchain uses the
    /// conventional `"NOCK"` symbol or an asset contract address).
    pub asset: String,
    /// Base58 public-key hash of the payee. For `exact`/`nockchain`, this
    /// MUST match `Authorization.to`.
    pub pay_to: String,
    /// Hard upper bound on time the server will wait for settlement,
    /// in seconds.
    pub max_timeout_seconds: u64,
    /// Optional human-readable description of the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional media type of the resource's response body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Optional JSON Schema describing the response shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Scheme-/network-specific extra fields (e.g. `minFee`, SIWN block).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    /// v2 extension blocks, keyed by extension name (e.g. `"bazaar"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
}

// ---------------------------------------------------------------------------
// PaymentRequired — server → client 402 envelope
// ---------------------------------------------------------------------------

/// The 402 response body the server ships when a resource requires payment.
/// Shape per `coinbase/x402:specs/extensions/bazaar.md` (top-level resource
/// info is promoted out of each `accepts[]` entry).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentRequired {
    /// Protocol version. MUST be `2`.
    pub x402_version: u32,
    /// Short human-readable error string (spec uses `"Payment required"`).
    pub error: String,
    /// Resource-level info (URL + optional description + mime type).
    pub resource: PaymentResource,
    /// One or more payment options the client may choose between.
    pub accepts: Vec<PaymentRequirements>,
    /// v2 extension blocks, keyed by extension name (e.g. `"bazaar"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
}

/// Resource-level metadata carried on [`PaymentRequired`]. Only `url` is
/// required by the upstream bazaar.md examples; `description`/`mimeType`
/// are optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentResource {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

// ---------------------------------------------------------------------------
// PaymentPayload — client → server, carried in PAYMENT-SIGNATURE header
// ---------------------------------------------------------------------------

/// Top-level envelope the client ships back to the server after signing.
/// Generic over `P`, the scheme-specific `payload` variant. The
/// network-neutral form (`P = serde_json::Value`) is the default; Nockchain
/// code uses [`crate::nockchain::ExactNockchainPayload`] behind the
/// `nockchain` feature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaymentPayload<P = Value> {
    /// Protocol version. MUST be `2` for the PR #102 spec.
    pub x402_version: u32,
    /// MUST match the selected `PaymentRequirements.scheme`.
    pub scheme: String,
    /// MUST match the selected `PaymentRequirements.network`.
    pub network: String,
    /// Scheme- and network-specific payload. See
    /// [`crate::nockchain::ExactNockchainPayload`] for the
    /// `(exact, nockchain:*)` variant.
    pub payload: P,
    /// Extension blocks echoed from `PaymentRequirements.extensions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
}

// ---------------------------------------------------------------------------
// Authorization — the signed inner struct
// ---------------------------------------------------------------------------

/// The authorization object the payer signs. Fully commits the facilitator
/// non-custodially — recipient, amount, fee, change address, and the exact
/// input notes are all pinned before signing.
///
/// Shape per PR #102 `05-payment-payload.md §5.3.2`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Authorization {
    /// Base58 public-key hash of the payer. MUST correspond to
    /// [`SchnorrSignatureJson::pubkey`] via `Tip5(pubkey) == from`.
    pub from: String,
    /// Base58 public-key hash of the payee. MUST match
    /// `PaymentRequirements.payTo`.
    pub to: String,
    /// Amount to transfer (nicks as decimal string).
    pub value: String,
    /// Transaction fee (nicks as decimal string).
    pub fee: String,
    /// Unique nonce for replay protection (base58-encoded Tip5 hash).
    pub nonce: String,
    /// Unix timestamp (seconds). Payment is invalid before this time.
    pub valid_after: u64,
    /// Unix timestamp (seconds). Payment is invalid after this time.
    pub valid_before: u64,
    /// Input notes (UTXOs) being spent.
    pub notes: Vec<NoteRef>,
    /// Base58 PKH to receive change (if any).
    pub change_address: String,
}

/// Reference to a specific Nockchain note (UTXO).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteRef {
    /// Two-hash note name.
    pub name: NoteName,
    /// The amount held by this note (nicks as decimal string). The
    /// facilitator verifies this against the on-chain state.
    pub assets: String,
    /// Lock type the note is currently held under. Determines the
    /// `SpendCondition` the wallet uses to build the witness's
    /// lock-merkle-proof at sign time. Defaults to
    /// [`NoteLock::SimplePkh`] so payloads predating the Phase-6 lift
    /// (which all assumed simple-PKH inputs) decode unchanged.
    ///
    /// Phase-6 / ADR-0017 finding: coinbase-locked notes carry an
    /// extra timelock primitive so their lock-root differs from a
    /// simple-PKH note's; without this metadata `authorize_and_sign`
    /// builds the wrong witness and the chain silently drops the tx.
    /// This is a Nockchain-local proposed-upstream extension to
    /// `05-payload.md` (governing constraint: ADR-0012 spec-snapshot
    /// pinning).
    ///
    /// Serialization skips the field when it equals the default
    /// (`SimplePkh`) so prior-Phase-6 wire shapes round-trip byte
    /// identically — only `CoinbasePkh` payloads emit the field. This
    /// also keeps the §5.4.1 golden vectors and any pinned envelopes
    /// produced by older signers byte-stable.
    #[serde(default, skip_serializing_if = "NoteLock::is_simple_pkh")]
    pub lock: NoteLock,
}

/// Lock type a [`NoteRef`] is held under at sign time.
///
/// Mirrors the chain-side dispatch in
/// `nockchain_types::tx_engine::v1::tx::SpendCondition::{simple_pkh,
/// coinbase_pkh}` — the wallet's witness must present the matching
/// `SpendCondition` for chain consensus to accept the spend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NoteLock {
    /// Plain pay-to-pubkey-hash. The default; what every pre-Phase-6
    /// payload assumed.
    #[default]
    SimplePkh,
    /// Coinbase-locked note with a relative-block-height timelock
    /// minimum. `timelock_min` MUST equal the network's
    /// `coinbase_timelock_min` chain constant (fakenet=1, mainnet=100
    /// per `nockchain_types::blockchain_constants`).
    CoinbasePkh { timelock_min: u64 },
}

impl NoteLock {
    /// Predicate for `serde(skip_serializing_if = ...)`. True when the
    /// lock equals the default (`SimplePkh`); used to keep the wire
    /// shape byte-identical for prior-Phase-6 payloads.
    pub fn is_simple_pkh(&self) -> bool {
        matches!(self, NoteLock::SimplePkh)
    }
}

/// Two-hash Nockchain note name (`first`, `last` Tip5 hashes as base58).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteName {
    pub first: String,
    pub last: String,
}

// ---------------------------------------------------------------------------
// SchnorrSignatureJson — base58 pubkey + 8×8 Belt signature
// ---------------------------------------------------------------------------

/// JSON encoding of a Schnorr-over-Cheetah signature together with the
/// signer's public key. Shape per `05-payment-payload.md §5.3.1`.
///
/// The `schnorr.chal` and `schnorr.sig` arrays are exactly 8 Belt
/// (base-field) values, transported as decimal strings to preserve u64
/// precision across JSON parsers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchnorrSignatureJson {
    /// Base58-encoded Schnorr (Cheetah) public key of the payer.
    pub pubkey: String,
    /// Challenge + signature, each 8 Belt values as decimal strings.
    pub schnorr: SchnorrPair,
}

/// Inner `{chal, sig}` block of [`SchnorrSignatureJson`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchnorrPair {
    /// Challenge hash — 8 Belt values as decimal strings.
    pub chal: [String; 8],
    /// Signature scalar — 8 Belt values as decimal strings.
    pub sig: [String; 8],
}

impl SchnorrSignatureJson {
    /// Phase-2 stub signature: every Belt position in both `chal` and
    /// `sig` is the literal `"0"`. Paired with the supplied `pubkey`
    /// string (typically the signer's base58-encoded public key, zeros
    /// for the stub signer).
    ///
    /// Used by `x402_client::StubSigner` to exercise the x402 envelope
    /// end-to-end before real Schnorr-over-Cheetah signing lands in
    /// Phase 3.
    pub fn all_zero(pubkey: impl Into<String>) -> Self {
        Self {
            pubkey: pubkey.into(),
            schnorr: SchnorrPair {
                chal: std::array::from_fn(|_| "0".to_string()),
                sig: std::array::from_fn(|_| "0".to_string()),
            },
        }
    }

    /// True when every Belt position in both `chal` and `sig` is the
    /// literal `"0"`. The Phase-2 facilitator uses this as its stub
    /// verification predicate; real Schnorr verification replaces it in
    /// Phase 3.
    pub fn is_all_zero(&self) -> bool {
        self.schnorr.chal.iter().all(|s| s == "0")
            && self.schnorr.sig.iter().all(|s| s == "0")
    }
}

// ---------------------------------------------------------------------------
// ExtensionResponsesHeader — base64'd JSON in the EXTENSION-RESPONSES header
// ---------------------------------------------------------------------------

/// Decoded shape of the `EXTENSION-RESPONSES` HTTP header that a facilitator
/// MAY append to `/verify` / `/settle` responses. Keyed by extension name.
/// Per `bazaar.md §Verify and Settlement Response Header`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionResponsesHeader {
    /// Outcome reported by the `bazaar` discovery extension, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bazaar: Option<BazaarExtensionResponse>,
    /// Responses from other extensions we don't have typed support for yet.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// `bazaar` key of [`ExtensionResponsesHeader`]. Status-plus-reason pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BazaarExtensionResponse {
    pub status: BazaarExtensionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_reason: Option<String>,
}

/// Three-state outcome of a bazaar cataloging attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BazaarExtensionStatus {
    Success,
    Processing,
    Rejected,
}
