//! Client-side helpers for the x402 protocol and its Bazaar extension.
//!
//! Provides:
//! - [`BazaarClient`] — typed client for the upstream-canonical
//!   `GET /discovery/resources` API.
//! - [`Signer`] trait — the extensibility seam that keeps this crate
//!   network-neutral. A Nockchain implementation lives in
//!   `x402-nockchain-crypto`; future EVM/SVM adapters would provide their
//!   own [`Signer`] impl without touching this crate.
//!
//! Reference: `coinbase/x402:specs/extensions/bazaar.md` (discovery API)
//! and PR #102 `specs/x402/05-payment-payload.md` (signing).

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use x402_types::{DiscoveryResourcesResponse, ListDiscoveryResourcesParams};

/// Trait that produces a payment-authorization signature over a canonical
/// auth-message byte string. Network adapters implement this.
///
/// M3 scope (per `vesl-agent/docs/plans/phase-3-real-signing-siwn.md`) fleshes
/// out the return type to carry structured signature material
/// (`SchnorrSignatureJson` from [`x402_types::payment`]) rather than raw bytes.
/// At M0 this is a minimal surface kept trivial on purpose.
#[async_trait]
pub trait Signer: Send + Sync {
    /// Sign the canonical authorization-message bytes. The caller is
    /// responsible for canonicalization (domain separator, Tip5 digest, etc.);
    /// the signer only applies the curve signature.
    async fn sign(&self, canonical_auth: &[u8]) -> Result<Vec<u8>>;

    /// Return the public key bytes associated with this signer (encoding is
    /// implementation-defined: Nockchain uses base58 Cheetah pubkeys).
    fn public_key(&self) -> Vec<u8>;
}

/// Typed client for the upstream-canonical `/discovery/resources` API.
///
/// Auth is intentionally pluggable but not yet wired — per
/// `BAZAAR_UPSTREAM_NOTES.md §3` the upstream `createAuthHeaders("discovery")`
/// hook is implementation-defined. On Nockchain this will be SIWN (wired in
/// M3 per `vesl-agent/docs/plans/phase-3-real-signing-siwn.md`).
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

fn percent_encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
