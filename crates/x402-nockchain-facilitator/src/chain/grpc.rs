//! gRPC-backed [`super::ChainClient`] wrapping
//! `nockchain_client_rs::ChainClient`.
//!
//! Path-2B settlement (per ADR-0010): the wallet ships a fully-signed
//! `RawTx` JSON-surrogate inside the x402
//! [`x402_types::payment::PaymentPayload`]. The facilitator's job here
//! is bounded: pull the [`x402_types::nockchain::SignedRawTx`] out of
//! the envelope, decode it back to a chain-typed
//! [`RawTx`](nockchain_types::tx_engine::v1::RawTx) via
//! [`x402_nockchain_wallet_client::raw_tx_from_surrogate`], submit it
//! through `nockchain_client_rs`, and poll for inclusion.
//!
//! The polling cadence comes from the underlying
//! [`nockchain_client_rs::ChainClient::wait_for_acceptance`]
//! implementation, which itself uses the `WAIT_BLOCKS_TIMEOUT`-shaped
//! interval/deadline pair from `vesl-core`'s harness — the convention
//! the phase doc explicitly calls out.

use std::sync::Arc;

use async_trait::async_trait;
use nockchain_client_rs::{ChainClient as NockChainClient, ChainConfig};
use serde_json::Value;
use tokio::sync::Mutex;
use x402_nockchain_wallet_client::raw_tx_from_surrogate;
use x402_types::facilitator::SettleRequest;
use x402_types::nockchain::SignedRawTx;

use super::{ChainClient, ChainError, ChainSettle, ChainStatus};

/// Wraps `nockchain_client_rs::ChainClient` behind a `Mutex` since that
/// API is `&mut self`. Cheap to clone — internal state is an `Arc`.
#[derive(Clone)]
pub struct GrpcChainClient {
    inner: Arc<Mutex<NockChainClient>>,
    #[allow(dead_code)]
    config: ChainConfig,
}

impl GrpcChainClient {
    /// Connect to a Nockchain public gRPC endpoint. Defaults to
    /// `http://localhost:9090` — override via `ChainConfig::local`.
    pub async fn connect(config: ChainConfig) -> Result<Self, ChainError> {
        let inner = NockChainClient::connect(config.clone())
            .await
            .map_err(|e| ChainError::Transport(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            config,
        })
    }

    /// Underlying chain-client handle for callers that need richer access
    /// (balance queries, direct polling). Held behind a mutex.
    pub fn inner(&self) -> Arc<Mutex<NockChainClient>> {
        self.inner.clone()
    }
}

#[async_trait]
impl ChainClient for GrpcChainClient {
    async fn settle(&self, req: &SettleRequest) -> Result<ChainSettle, ChainError> {
        // 1. Pull the SignedRawTx surrogate out of the typed payload.
        let signed = extract_signed_raw_tx(req)?;

        // 2. Decode the surrogate back to a chain-typed RawTx.
        let raw = raw_tx_from_surrogate(&signed).map_err(|e| {
            ChainError::Rejected(format!("decode SignedRawTx surrogate: {e}"))
        })?;
        let tx_id = signed.tx_id.clone();

        // 3. Submit + wait for acceptance.
        let mut client = self.inner.lock().await;
        let accepted = client
            .submit_and_wait(raw, &tx_id)
            .await
            .map_err(|e| ChainError::Transport(format!("submit_and_wait: {e}")))?;

        let status = if accepted {
            ChainStatus::Accepted
        } else {
            // Per `bazaar.md`, polling-timeout maps to
            // `EXTENSION-RESPONSES: {"bazaar": {"status": "processing"}}`.
            ChainStatus::Processing
        };

        Ok(ChainSettle {
            tx_id,
            block_height: None,
            status,
        })
    }
}

/// Unpack `req.payload.payload.signed_raw_tx` from the network-neutral
/// `serde_json::Value` envelope. The error path is meaningful: a
/// missing or malformed surrogate is the operational signal that the
/// client is on the legacy envelope-only path-2A flow rather than
/// path-2B.
fn extract_signed_raw_tx(req: &SettleRequest) -> Result<SignedRawTx, ChainError> {
    let raw_field: &Value = req.payload.payload.get("signedRawTx").ok_or_else(|| {
        ChainError::TxConstructionUnavailable(
            "PaymentPayload.payload.signedRawTx missing — clients must use the \
             path-2B WalletBackend flow (see docs/wallet-integration.md)"
                .into(),
        )
    })?;
    serde_json::from_value::<SignedRawTx>(raw_field.clone()).map_err(|e| {
        ChainError::Rejected(format!(
            "PaymentPayload.payload.signedRawTx is malformed: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use x402_types::facilitator::SettleRequest;
    use x402_types::payment::{PaymentPayload, PaymentRequirements};

    fn fixture_request(payload_value: Value) -> SettleRequest {
        SettleRequest {
            payload: PaymentPayload {
                x402_version: 2,
                scheme: "exact".to_string(),
                network: "nockchain:fakenet".to_string(),
                payload: payload_value,
                extensions: None,
            },
            requirements: PaymentRequirements {
                scheme: "exact".to_string(),
                network: "nockchain:fakenet".to_string(),
                max_amount_required: "1".to_string(),
                resource: "/x".to_string(),
                asset: "NOCK".to_string(),
                pay_to: "anywhere".to_string(),
                max_timeout_seconds: 30,
                description: None,
                mime_type: None,
                output_schema: None,
                extra: None,
                extensions: None,
            },
        }
    }

    #[test]
    fn extract_returns_unimplemented_when_signed_raw_tx_missing() {
        let req = fixture_request(json!({ "authorization": {} }));
        let err = extract_signed_raw_tx(&req).unwrap_err();
        assert!(matches!(err, ChainError::TxConstructionUnavailable(_)));
    }

    #[test]
    fn extract_returns_rejected_when_signed_raw_tx_malformed() {
        let req = fixture_request(json!({
            "authorization": {},
            "signedRawTx": { "this": "is not the surrogate shape" }
        }));
        let err = extract_signed_raw_tx(&req).unwrap_err();
        assert!(matches!(err, ChainError::Rejected(_)));
    }
}
