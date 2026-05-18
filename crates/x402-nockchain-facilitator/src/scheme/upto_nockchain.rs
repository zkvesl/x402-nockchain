//! `(upto, nockchain:*)` handler — bounty-style settlement where the
//! client may pay any amount *not exceeding* `max_amount_required`.
//!
//! On the wire this scheme is structurally identical to
//! `(exact, nockchain:*)` (`UptoNockchainPayload` is a type alias of
//! [`ExactNockchainPayload`]); the spec does not pin a different
//! payload shape. The verifier's accept condition is also identical
//! to `exact`'s — `auth.value <= requirements.max_amount_required` —
//! because both schemes enforce the same upper bound. The semantic
//! difference is the *expectation*: `exact` callers expect
//! `auth.value == max_amount_required`, while `upto` callers may
//! submit any value at or below the cap (including zero, which is
//! useful for free-tier bounty acks). See ADR-0020 for the rationale
//! + the proposed-upstream filing posture.

use async_trait::async_trait;
use serde_json::Value;
use x402_nockchain_crypto::NockchainVerifier;
use x402_types::facilitator::SettleRequest;
use x402_types::nockchain::UptoNockchainPayload;
use x402_types::payment::{Authorization, PaymentPayload, PaymentRequirements};

use crate::chain::{ChainClient, ChainError, ChainSettle};
use crate::error::VerifyError;

use super::exact_nockchain::{check_asset, check_max_amount};
use super::SchemeHandler;

#[derive(Debug, Default, Clone, Copy)]
pub struct UptoNockchainHandler;

#[async_trait]
impl SchemeHandler for UptoNockchainHandler {
    fn scheme(&self) -> &'static str {
        "upto"
    }

    fn network_prefix(&self) -> &'static str {
        "nockchain:"
    }

    fn extract_authorization(
        &self,
        payload: &PaymentPayload<Value>,
    ) -> Result<Authorization, VerifyError> {
        let nock: UptoNockchainPayload = serde_json::from_value(payload.payload.clone())
            .map_err(|e| VerifyError::SpecCoded {
                code: "invalid_payload".into(),
                message: format!("payload decode failed: {e}"),
            })?;
        Ok(nock.authorization)
    }

    async fn verify(
        &self,
        payload: &PaymentPayload<Value>,
        requirements: &PaymentRequirements,
    ) -> Result<(), VerifyError> {
        let nock: UptoNockchainPayload = serde_json::from_value(payload.payload.clone())
            .map_err(|e| VerifyError::SpecCoded {
                code: "invalid_payload".into(),
                message: format!("payload decode failed: {e}"),
            })?;

        if nock.authorization.to != requirements.pay_to {
            return Err(VerifyError::SpecCoded {
                code: "invalid_recipient".into(),
                message: "authorization.to does not match requirements.payTo".into(),
            });
        }

        NockchainVerifier::new()
            .verify_payment(payload, requirements)
            .map_err(|e| VerifyError::SpecCoded {
                code: "invalid_signature".into(),
                message: e.to_string(),
            })?;

        check_max_amount(&nock.authorization, requirements)?;
        check_asset(requirements)?;

        Ok(())
    }

    async fn settle(
        &self,
        chain: &dyn ChainClient,
        request: &SettleRequest,
    ) -> Result<ChainSettle, ChainError> {
        chain.settle(request).await
    }
}
