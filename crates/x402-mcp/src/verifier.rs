//! Payment-verification abstraction for the MCP router.
//!
//! The registry is deliberately network-free; the router plugs in a
//! `PaymentVerifier` implementation that decides which requests to admit.
//!
//! Two impls ship:
//!
//! - [`AlwaysAcceptVerifier`] — every payload is admitted. Only for tests
//!   and early-phase demos.
//! - [`RemoteFacilitatorVerifier`] — POSTs to a facilitator `/verify`
//!   URL; defers all crypto + envelope checks to the facilitator.

use async_trait::async_trait;
use serde_json::Value;
use x402_types::facilitator::{VerifyRequest, VerifyResponse};
use x402_types::payment::{PaymentPayload, PaymentRequirements};

/// Reason a payment was rejected. Propagates to the caller as a 402 with
/// the bazaar extension echoed back.
#[derive(Debug, thiserror::Error)]
pub enum VerifyFailure {
    /// The payment payload was rejected by the verifier (bad signature,
    /// mismatched scheme, etc).
    #[error("payment rejected: {0}")]
    Rejected(String),
    /// The verifier couldn't reach the facilitator or decode its reply.
    #[error("verifier transport error: {0}")]
    Transport(String),
}

#[async_trait]
pub trait PaymentVerifier: Send + Sync {
    async fn verify(
        &self,
        payload: &PaymentPayload<Value>,
        requirements: &PaymentRequirements,
    ) -> Result<(), VerifyFailure>;
}

/// Admits every payment. Only for tests and scaffolding demos.
#[derive(Debug, Default, Clone)]
pub struct AlwaysAcceptVerifier;

#[async_trait]
impl PaymentVerifier for AlwaysAcceptVerifier {
    async fn verify(
        &self,
        _payload: &PaymentPayload<Value>,
        _requirements: &PaymentRequirements,
    ) -> Result<(), VerifyFailure> {
        Ok(())
    }
}

/// Defers verification to a remote facilitator via `POST {base}/verify`.
#[derive(Debug, Clone)]
pub struct RemoteFacilitatorVerifier {
    base_url: String,
    http: reqwest::Client,
}

impl RemoteFacilitatorVerifier {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl PaymentVerifier for RemoteFacilitatorVerifier {
    async fn verify(
        &self,
        payload: &PaymentPayload<Value>,
        requirements: &PaymentRequirements,
    ) -> Result<(), VerifyFailure> {
        let req = VerifyRequest {
            payload: payload.clone(),
            requirements: requirements.clone(),
        };
        let url = format!("{}/verify", self.base_url.trim_end_matches('/'));
        let resp: VerifyResponse = self
            .http
            .post(url)
            .json(&req)
            .send()
            .await
            .map_err(|e| VerifyFailure::Transport(e.to_string()))?
            .json()
            .await
            .map_err(|e| VerifyFailure::Transport(e.to_string()))?;

        if resp.valid {
            Ok(())
        } else {
            let msg = resp
                .error
                .map(|e| format!("{}: {}", e.code, e.message))
                .unwrap_or_else(|| "verifier returned valid=false with no error".into());
            Err(VerifyFailure::Rejected(msg))
        }
    }
}
