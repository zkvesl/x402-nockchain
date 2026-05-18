//! Client-side helpers for the x402 protocol and its Bazaar extension.
//!
//! Exports:
//! - [`Signer`] — the signing seam. Returns a [`SchnorrSignatureJson`]
//!   directly so network-specific signers (e.g. `x402_nockchain_crypto::
//!   NockchainSigner`) can plug in without re-serialization.
//! - [`StubSigner`] — Phase-2 holdover that returns all-zero signatures;
//!   still used by `e2e_demo` for a wire-shape smoke test, but rejected
//!   by the Phase-3 `NockchainVerifier`.
//! - [`BazaarClient`] — typed client for `GET /discovery/resources`.
//! - [`X402Client`] — high-level orchestrator that drives the
//!   402 → verify → retry loop.
//!
//! Reference: `coinbase/x402:specs/extensions/bazaar.md` (discovery API)
//! and PR #102 `specs/x402/{05-payment-payload.md, 06-facilitator.md}`.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctest;

pub mod policy;

pub use policy::{PolicyDenied, PolicyEnforcedSigner, PolicyFn};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use reqwest::{
    header::{HeaderName, HeaderValue},
    StatusCode,
};
use serde_json::Value;
use x402_types::facilitator::{VerifyRequest, VerifyResponse};
use x402_types::nockchain::{ExactNockchainPayload, SignedRawTx};
use x402_types::payment::{
    Authorization, ExtensionResponsesHeader, NoteName, NoteRef, PaymentPayload, PaymentRequired,
    PaymentRequirements, SchnorrSignatureJson,
};
use x402_types::{DiscoveryResourcesResponse, ListDiscoveryResourcesParams};

// ---------------------------------------------------------------------------
// Signer trait + phase-2 stub impl
// ---------------------------------------------------------------------------

/// Signer trait for the `(exact, nockchain:*)` scheme.
///
/// Takes a full [`Authorization`] so the implementer can apply the exact
/// Tip5 sponge specified in `05-payload.md §5.4.1`. Also takes the
/// matching [`PaymentRequirements`] so a wrapper signer (e.g.
/// [`policy::PolicyEnforcedSigner`]) can refuse to produce a signature
/// when caller-side policy rejects the requirements — without needing to
/// re-derive them from the authorization. The spec digest itself is
/// still over [`Authorization`] alone (per `06-facilitator.md §6.4`);
/// the second argument is purely a signer-side policy seam.
///
/// Returns the on-wire [`SchnorrSignatureJson`] directly — no
/// intermediate byte-serialization seam that could drift between
/// signer + verifier.
#[async_trait]
pub trait Signer: Send + Sync {
    /// Sign a complete `Authorization` and return the on-wire envelope.
    /// Implementations MAY inspect `requirements` to decide whether to
    /// sign; concrete signers (e.g. `NockchainSigner`) ignore it.
    async fn sign_authorization(
        &self,
        auth: &Authorization,
        requirements: &PaymentRequirements,
    ) -> Result<SchnorrSignatureJson>;

    /// Identifier to use as [`Authorization::from`]. For Nockchain this
    /// is the base58 Cheetah public key; for stubs it is a placeholder.
    fn from_identifier(&self) -> String;
}

/// Phase-2 holdover signer that returns an all-zero
/// [`SchnorrSignatureJson`] regardless of input. The Phase-3
/// `NockchainVerifier` rejects these, so this signer is kept only for
/// wire-shape smoke tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubSigner;

#[async_trait]
impl Signer for StubSigner {
    async fn sign_authorization(
        &self,
        _auth: &Authorization,
        _requirements: &PaymentRequirements,
    ) -> Result<SchnorrSignatureJson> {
        Ok(SchnorrSignatureJson::all_zero("stub-pubkey"))
    }

    fn from_identifier(&self) -> String { "stub-from".into() }
}

// ---------------------------------------------------------------------------
// WalletBackend trait — path-2B authorize-and-sign contract
// ---------------------------------------------------------------------------

/// Combined result of a wallet's authorize-and-sign call: the §5.4.1
/// envelope signature plus the fully-signed chain-ready `RawTx`.
#[derive(Debug, Clone)]
pub struct AuthorizedPayment {
    pub envelope_signature: SchnorrSignatureJson,
    pub signed_raw_tx: SignedRawTx,
}

/// Errors a [`WalletBackend`] may raise. Implementations map their
/// transport-specific failures into these variants; the client-side
/// caller surfaces them to the user or retries as appropriate.
#[derive(Debug, thiserror::Error)]
pub enum WalletBackendError {
    /// RPC / IPC / message-passing transport to the wallet failed.
    #[error("wallet transport error: {0}")]
    Transport(String),
    /// Wallet has no UTXO set that satisfies `value + fee`.
    #[error("insufficient funds: {0}")]
    InsufficientFunds(String),
    /// User (or daemon policy) declined to sign.
    #[error("user rejected signing: {0}")]
    Rejected(String),
    /// Wallet doesn't support the requested transaction shape
    /// (legacy wallet, v0-only, etc).
    #[error("wallet does not support this transaction shape: {0}")]
    Unsupported(String),
    /// Catch-all for wallet-specific failures that don't fit the
    /// variants above. Include the wallet name + a diagnostic string.
    #[error("{0}")]
    Other(String),
}

/// The path-2B wallet contract (per ADR-0010 + `docs/wallet-integration.md`).
///
/// A `WalletBackend` takes an `Authorization` struct, pairs it with the
/// payer's private key, and produces:
///   1. The §5.4.1 envelope signature the facilitator verifies.
///   2. A fully-signed on-chain `RawTx` the facilitator submits.
///
/// Implementations cover whichever wallet holds the key: daemons
/// (`nockchain-wallet` on gRPC :5555), browser extensions (iris-wallet
/// via `iris-sdk`), hardware wallets, etc. The facilitator never talks
/// to a `WalletBackend` — only the client does.
#[async_trait]
pub trait WalletBackend: Send + Sync {
    /// base58 PKH of the payer's pubkey. Must equal
    /// `Authorization.from` for any `auth` this wallet signs.
    fn payer_pkh(&self) -> String;

    /// base58 of the full 97-byte Cheetah pubkey. Carried on
    /// `SchnorrSignatureJson.pubkey` so the facilitator can verify
    /// both the envelope signature and the `Tip5(pubkey) == from`
    /// binding.
    fn payer_pubkey_base58(&self) -> String;

    /// Authorize + sign. See `docs/wallet-integration.md §1` for the
    /// contract each implementation fulfils.
    ///
    /// Mirrors [`Signer::sign_authorization`] in taking
    /// [`PaymentRequirements`] alongside the [`Authorization`] so wrapper
    /// wallets can apply caller-side policy (refuse a payment whose
    /// requirements violate operator settings) before driving the
    /// underlying signing flow. Concrete wallets ignore `requirements`
    /// — the spec digest is over `auth` alone (per
    /// `06-facilitator.md §6.4`).
    async fn authorize_and_sign(
        &self,
        auth: &Authorization,
        requirements: &PaymentRequirements,
    ) -> Result<AuthorizedPayment, WalletBackendError>;
}

/// Minimal in-memory wallet for tests and Phase-4 scaffolding demos.
/// Returns all-zero signatures and a skeletal `SignedRawTx`; the
/// facilitator's Phase-3 verifier rejects the envelope signature, so
/// this is strictly wire-shape-only.
#[derive(Debug, Default, Clone)]
pub struct StubWalletBackend {
    pub pkh: String,
    pub pubkey_base58: String,
}

impl StubWalletBackend {
    pub fn new(pkh: impl Into<String>, pubkey_base58: impl Into<String>) -> Self {
        Self {
            pkh: pkh.into(),
            pubkey_base58: pubkey_base58.into(),
        }
    }
}

#[async_trait]
impl WalletBackend for StubWalletBackend {
    fn payer_pkh(&self) -> String { self.pkh.clone() }
    fn payer_pubkey_base58(&self) -> String { self.pubkey_base58.clone() }

    async fn authorize_and_sign(
        &self,
        auth: &Authorization,
        _requirements: &PaymentRequirements,
    ) -> Result<AuthorizedPayment, WalletBackendError> {
        use x402_types::nockchain::{
            SignedNoteName, SignedPkhSignatureEntry, SignedRawTx, SignedSchnorrAtoms, SignedSeed,
            SignedSpend, SignedSpendEntry, SignedWitness,
        };
        let zero_8 = || ["0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(),
                         "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string()];
        let envelope_signature = SchnorrSignatureJson::all_zero(&self.pubkey_base58);
        let signed_raw_tx = SignedRawTx {
            version: "v1".into(),
            tx_id: format!("stub-txid-{}", auth.nonce),
            spends: auth
                .notes
                .iter()
                .map(|n| SignedSpendEntry {
                    name: SignedNoteName {
                        first: n.name.first.clone(),
                        last: n.name.last.clone(),
                    },
                    spend: SignedSpend {
                        witness: SignedWitness {
                            lock_merkle_proof_noun: "stub".into(),
                            pkh_signature: vec![SignedPkhSignatureEntry {
                                hash: self.pkh.clone(),
                                pubkey: self.pubkey_base58.clone(),
                                signature: SignedSchnorrAtoms {
                                    chal: zero_8(),
                                    sig: zero_8(),
                                },
                            }],
                            hax: vec![],
                            tim: 0,
                        },
                        seeds: vec![SignedSeed {
                            output_source_noun: None,
                            lock_root: auth.to.clone(),
                            note_data_noun: None,
                            gift: auth.value.clone(),
                            parent_hash: n.name.first.clone(),
                        }],
                        fee: auth.fee.clone(),
                    },
                })
                .collect(),
        };
        Ok(AuthorizedPayment {
            envelope_signature,
            signed_raw_tx,
        })
    }
}

// ---------------------------------------------------------------------------
// BazaarClient — typed /discovery/resources
// ---------------------------------------------------------------------------

/// Typed client for the upstream-canonical `/discovery/resources` API.
///
/// Auth is intentionally pluggable but not yet wired — the upstream
/// `createAuthHeaders("discovery")` hook is implementation-defined. On
/// Nockchain this will be SIWN (see `docs/specs-snapshot/11-extensions.md
/// §11.2`).
pub struct BazaarClient {
    base_url: String,
    http: reqwest::Client,
}

impl BazaarClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn list_resources(
        &self,
        params: ListDiscoveryResourcesParams,
    ) -> Result<DiscoveryResourcesResponse> {
        let mut url = format!("{}/discovery/resources", self.base_url);
        let mut qs = Vec::new();
        if let Some(k) = &params.kind {
            qs.push(format!("type={}", percent_encode(k)));
        }
        if let Some(l) = params.limit {
            qs.push(format!("limit={}", l));
        }
        if let Some(o) = params.offset {
            qs.push(format!("offset={}", o));
        }
        if let Some(n) = &params.network {
            qs.push(format!("network={}", percent_encode(n)));
        }
        if let Some(s) = &params.scheme {
            qs.push(format!("scheme={}", percent_encode(s)));
        }
        if !qs.is_empty() {
            url.push('?');
            url.push_str(&qs.join("&"));
        }

        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "BazaarClient::list_resources failed ({}): {}",
                status,
                body
            ));
        }
        Ok(resp.json::<DiscoveryResourcesResponse>().await?)
    }
}

// ---------------------------------------------------------------------------
// X402Client — high-level 402 flow orchestrator
// ---------------------------------------------------------------------------

/// High-level client that drives the x402 402 → verify → retry loop.
/// Phase-2 scope: GET a resource; if the server returns 402, echo the
/// advertised bazaar extension into a stub-signed `PaymentPayload`,
/// POST it to the facilitator's `/verify`, and (if verify passes) retry
/// the resource request with the `PAYMENT-SIGNATURE` header attached.
pub struct X402Client {
    http: reqwest::Client,
}

impl Default for X402Client {
    fn default() -> Self {
        Self::new()
    }
}

impl X402Client {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// GET the resource once and parse any 402 response into a typed
    /// [`PaymentRequired`]. Returns `Ok(Err(response))` for non-402
    /// status codes so callers can short-circuit without allocating a
    /// PaymentRequired.
    pub async fn get_payment_required(&self, url: &str) -> Result<PaymentRequired> {
        let resp = self.http.get(url).send().await.context("GET resource")?;
        if resp.status() != StatusCode::PAYMENT_REQUIRED {
            return Err(anyhow!(
                "expected 402 Payment Required, got {} from {}",
                resp.status(),
                url
            ));
        }
        let envelope: PaymentRequired = resp
            .json()
            .await
            .context("decode PaymentRequired JSON body")?;
        Ok(envelope)
    }

    /// Call a facilitator's `/verify` endpoint with the given payload and
    /// requirements. Returns the decoded `VerifyResponse` and, if the
    /// facilitator included an `EXTENSION-RESPONSES` header, its decoded
    /// contents.
    pub async fn verify(
        &self,
        facilitator_url: &str,
        payload: &PaymentPayload<Value>,
        requirements: &PaymentRequirements,
    ) -> Result<(VerifyResponse, Option<ExtensionResponsesHeader>)> {
        let body = VerifyRequest {
            payload: payload.clone(),
            requirements: requirements.clone(),
        };
        let resp = self
            .http
            .post(format!("{}/verify", facilitator_url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .context("POST /verify")?;

        let status = resp.status();
        let ext_header = resp
            .headers()
            .get(HeaderName::from_static("extension-responses"))
            .cloned();
        let verify: VerifyResponse = resp.json().await.with_context(|| {
            format!("decode VerifyResponse JSON (status {})", status.as_u16())
        })?;

        let ext_decoded = ext_header.as_ref().and_then(decode_extension_header);

        Ok((verify, ext_decoded))
    }

    /// Convenience: full 402-flow round trip. GETs the resource, echoes
    /// the bazaar extension into a stub-signed payload, verifies with the
    /// facilitator, and (on success) retries the GET with the
    /// `PAYMENT-SIGNATURE` header attached. Returns the final response
    /// body as a `String`.
    pub async fn fetch_with_payment(
        &self,
        resource_url: &str,
        facilitator_url: &str,
        signer: &dyn Signer,
    ) -> Result<reqwest::Response> {
        let envelope = self.get_payment_required(resource_url).await?;
        let requirements = envelope
            .accepts
            .first()
            .ok_or_else(|| anyhow!("PaymentRequired.accepts is empty"))?
            .clone();

        let payload = build_exact_nockchain_payload(&requirements, signer, &envelope.extensions)
            .await?;

        let (verify, _) = self
            .verify(facilitator_url, &payload, &requirements)
            .await?;
        if !verify.valid {
            return Err(anyhow!(
                "facilitator rejected stub payload: {:?}",
                verify.error
            ));
        }

        let payload_header = encode_payment_signature_header(&payload)?;
        let resp = self
            .http
            .get(resource_url)
            .header(
                HeaderName::from_static("payment-signature"),
                HeaderValue::from_str(&payload_header)
                    .context("render PAYMENT-SIGNATURE header")?,
            )
            .send()
            .await
            .context("retry GET with PAYMENT-SIGNATURE")?;
        Ok(resp)
    }
}

// ---------------------------------------------------------------------------
// Payload builders — phase-2 stub for (exact, nockchain:*)
// ---------------------------------------------------------------------------

/// Build a phase-2 stub `PaymentPayload<Value>` for the `(exact,
/// nockchain:*)` scheme. The [`Signer`] is exercised (both `sign` and
/// `public_key` are called) but its output is discarded — the
/// `SchnorrSignatureJson` is always all-zero. Phase 3 replaces this with
/// real Schnorr-over-Cheetah signing.
pub async fn build_exact_nockchain_payload(
    requirements: &PaymentRequirements,
    signer: &dyn Signer,
    echo_extensions: &Option<std::collections::BTreeMap<String, Value>>,
) -> Result<PaymentPayload<Value>> {
    let from = signer.from_identifier();

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let valid_after = now.saturating_sub(5);
    let valid_before = now.saturating_add(requirements.max_timeout_seconds);

    let fee = requirements
        .extra
        .as_ref()
        .and_then(|e| e.get("minFee"))
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();

    let authorization = Authorization {
        from: from.clone(),
        to: requirements.pay_to.clone(),
        value: requirements.max_amount_required.clone(),
        fee,
        nonce: generate_nonce_base58(),
        valid_after,
        valid_before,
        notes: vec![NoteRef {
            name: NoteName {
                first: random_pkh_base58(),
                last: random_pkh_base58(),
            },
            assets: requirements.max_amount_required.clone(),
            lock: Default::default(),
        }],
        change_address: from,
    };

    let signature = signer.sign_authorization(&authorization, requirements).await?;

    // signed_raw_tx is None in this envelope-only builder. Callers
    // that need path-2B chain submission use
    // [`build_exact_nockchain_payload_with_wallet`] instead; the
    // facilitator's `/settle` requires the signed_raw_tx field to be
    // present before it can submit on-chain.
    let exact = ExactNockchainPayload {
        signature,
        authorization,
        signed_raw_tx: None,
    };
    let payload_value = serde_json::to_value(&exact).context("serialize ExactNockchainPayload")?;

    Ok(PaymentPayload {
        x402_version: 2,
        scheme: requirements.scheme.clone(),
        network: requirements.network.clone(),
        payload: payload_value,
        extensions: echo_extensions.clone(),
    })
}

/// Canonicalize an [`Authorization`] for debugging/inspection as a
/// sorted-key JSON byte string.
///
/// **This is not what gets signed.** The spec-faithful signing path is
/// `x402_nockchain_crypto::x402_sign_message_digest` (per
/// `docs/specs-snapshot/05-payload.md §5.4.1`), which runs a Tip5 sponge
/// over Belt-encoded authorization fields. This helper is kept because
/// the sorted-JSON form remains useful for request-logging and
/// deterministic hashing of unsigned envelopes (catalog upserts,
/// EXTENSION-RESPONSES headers, etc).
pub fn canonical_auth_bytes(auth: &Authorization) -> Result<Vec<u8>> {
    let v: Value = serde_json::to_value(auth).context("serialize auth to Value")?;
    serde_json::to_vec(&v).context("serialize canonical auth Value")
}

// ---------------------------------------------------------------------------
// Header codecs
// ---------------------------------------------------------------------------

/// Encode a `PaymentPayload` for the `PAYMENT-SIGNATURE` header
/// (standard base64 of the JSON, per spec).
pub fn encode_payment_signature_header(payload: &PaymentPayload<Value>) -> Result<String> {
    let json = serde_json::to_string(payload).context("serialize PaymentPayload")?;
    Ok(B64.encode(json))
}

/// Decode the facilitator's `EXTENSION-RESPONSES` header (base64'd JSON).
fn decode_extension_header(value: &HeaderValue) -> Option<ExtensionResponsesHeader> {
    let bytes = B64.decode(value.as_bytes()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Path-2B payload builder for `(exact, nockchain:*)`.
///
/// Delegates both the §5.4.1 envelope signature AND the on-chain
/// `RawTx` assembly to the provided `WalletBackend`. The wallet picks
/// the UTXOs, computes the chain `sig_hash`, signs, and returns a
/// `SignedRawTx` ready for submission. This builder:
///
///   1. Constructs the `Authorization` from the `PaymentRequirements`
///      plus the wallet's claimed PKH.
///   2. Calls [`WalletBackend::authorize_and_sign`] to get both
///      signatures in one round trip.
///   3. Assembles the `PaymentPayload` with `signed_raw_tx = Some(...)`.
///
/// Unlike [`build_exact_nockchain_payload`], the caller does not
/// pre-populate `Authorization.notes` — the wallet knows which notes
/// it owns and will fill that in before signing. The caller observes
/// the chosen notes on the returned payload.
pub async fn build_exact_nockchain_payload_with_wallet(
    requirements: &PaymentRequirements,
    wallet: &dyn WalletBackend,
    notes: Vec<NoteRef>,
    echo_extensions: &Option<std::collections::BTreeMap<String, Value>>,
) -> Result<PaymentPayload<Value>> {
    let from = wallet.payer_pkh();

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let valid_after = now.saturating_sub(5);
    let valid_before = now.saturating_add(requirements.max_timeout_seconds);

    let fee = requirements
        .extra
        .as_ref()
        .and_then(|e| e.get("minFee"))
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();

    let authorization = Authorization {
        from: from.clone(),
        to: requirements.pay_to.clone(),
        value: requirements.max_amount_required.clone(),
        fee,
        nonce: generate_nonce_base58(),
        valid_after,
        valid_before,
        notes,
        change_address: from,
    };

    let signed = wallet
        .authorize_and_sign(&authorization, requirements)
        .await
        .map_err(|e| anyhow!("wallet authorize_and_sign failed: {e}"))?;

    let exact = ExactNockchainPayload {
        signature: signed.envelope_signature,
        authorization,
        signed_raw_tx: Some(signed.signed_raw_tx),
    };
    let payload_value = serde_json::to_value(&exact).context("serialize ExactNockchainPayload")?;

    Ok(PaymentPayload {
        x402_version: 2,
        scheme: requirements.scheme.clone(),
        network: requirements.network.clone(),
        payload: payload_value,
        extensions: echo_extensions.clone(),
    })
}

/// Generate a nonce as base58-encoded 32 random bytes. 32 bytes always
/// decodes into a `< PRIME^5` value so the facilitator's
/// `x402_nockchain_crypto::base58_to_belts` cannot overflow.
///
/// This is intentionally a weaker nonce than the spec §5.5.1 Tip5
/// construction (which would bind the nonce to from / resource-url /
/// timestamp). The stronger form belongs in the Nockchain-specific
/// signer crate; the network-neutral client settles for cryptographic
/// randomness adequate for replay protection.
pub fn generate_nonce_base58() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    bs58::encode(&buf).into_string()
}

/// 32 random bytes as base58 — placeholder for a chain-queried note
/// name in callers that don't yet have UTXO data. Real clients replace
/// this by a `ChainClient::get_balance_by_pkh` lookup before signing.
pub fn random_pkh_base58() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    bs58::encode(&buf).into_string()
}

#[cfg(test)]
mod wallet_backend_tests {
    use super::*;

    #[tokio::test]
    async fn stub_wallet_builds_payload_with_signed_raw_tx() {
        let wallet = StubWalletBackend::new("stub-pkh", "stub-pubkey");
        let notes = vec![NoteRef {
            name: NoteName {
                first: "first-b58".into(),
                last: "last-b58".into(),
            },
            assets: "1000".into(),
            lock: Default::default(),
        }];
        let requirements = PaymentRequirements {
            scheme: "exact".into(),
            network: "nockchain:fakenet".into(),
            max_amount_required: "500".into(),
            resource: "/x".into(),
            asset: "NOCK".into(),
            pay_to: "payee".into(),
            max_timeout_seconds: 30,
            description: None,
            mime_type: None,
            output_schema: None,
            extra: Some(serde_json::json!({ "minFee": "10" })),
            extensions: None,
        };
        let payload = build_exact_nockchain_payload_with_wallet(
            &requirements,
            &wallet,
            notes.clone(),
            &None,
        )
        .await
        .expect("build should succeed");

        let exact: ExactNockchainPayload = serde_json::from_value(payload.payload).unwrap();
        assert_eq!(exact.authorization.from, "stub-pkh");
        assert_eq!(exact.authorization.to, "payee");
        assert_eq!(exact.authorization.notes.len(), 1);
        assert_eq!(exact.signature.pubkey, "stub-pubkey");
        let signed = exact.signed_raw_tx.expect("signed_raw_tx must be present");
        assert_eq!(signed.version, "v1");
        assert_eq!(signed.spends.len(), 1);
        assert_eq!(signed.spends[0].spend.fee, "10");
        assert_eq!(signed.spends[0].spend.seeds[0].gift, "500");
    }
}

fn percent_encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
