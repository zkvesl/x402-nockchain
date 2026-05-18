//! Deterministic stub implementation of [`ChainClient`]. Preserves the
//! Phase-2 synthetic-tx_id behaviour so the existing roundtrip tests
//! continue to pass without a live fakenet.

use async_trait::async_trait;
use x402_types::facilitator::SettleRequest;

use super::{ChainClient, ChainError, ChainSettle, ChainStatus};

/// Returns `ChainStatus::Broadcast` with a deterministic `tx_id` derived
/// from the authorization nonce. Never fails.
#[derive(Debug, Default, Clone)]
pub struct StubChainClient;

#[async_trait]
impl ChainClient for StubChainClient {
    async fn settle(&self, req: &SettleRequest) -> Result<ChainSettle, ChainError> {
        Ok(ChainSettle {
            tx_id: stub_tx_id(req),
            block_height: None,
            status: ChainStatus::Broadcast,
        })
    }
}

pub(crate) fn stub_tx_id(req: &SettleRequest) -> String {
    match req
        .payload
        .payload
        .get("authorization")
        .and_then(|a| a.get("nonce"))
        .and_then(|n| n.as_str())
    {
        Some(nonce) => format!("stub-tx-{nonce}"),
        None => "stub-tx-unknown-nonce".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use x402_types::facilitator::SettleRequest;
    use x402_types::payment::PaymentRequirements;

    fn fixture_request(nonce: &str) -> SettleRequest {
        SettleRequest {
            payload: x402_types::payment::PaymentPayload {
                x402_version: 2,
                scheme: "exact".to_string(),
                network: "nockchain:fakenet".to_string(),
                payload: json!({ "authorization": { "nonce": nonce } }),
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

    #[tokio::test]
    async fn stub_returns_broadcast_with_deterministic_tx_id() {
        let req = fixture_request("abc123");
        let out = StubChainClient.settle(&req).await.unwrap();
        assert_eq!(out.tx_id, "stub-tx-abc123");
        assert_eq!(out.status, ChainStatus::Broadcast);
        assert!(out.block_height.is_none());
    }

    #[tokio::test]
    async fn stub_tx_id_falls_back_when_nonce_missing() {
        let mut req = fixture_request("ignored");
        req.payload.payload = json!({ "authorization": {} });
        let out = StubChainClient.settle(&req).await.unwrap();
        assert_eq!(out.tx_id, "stub-tx-unknown-nonce");
    }
}
