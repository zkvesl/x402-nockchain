//! `(exact, nockchain:*)` handler — the pre-R1.2 default branch lifted
//! into the registry. Preserves R1.1's verifier-policy semantics
//! byte-identically; the only behavior change is that unknown
//! `(scheme, network)` pairs no longer fall through to a silent `Ok`
//! (per ADR-0020).

use async_trait::async_trait;
use serde_json::Value;
use x402_nockchain_crypto::NockchainVerifier;
use x402_types::facilitator::SettleRequest;
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{Authorization, PaymentPayload, PaymentRequirements};

use crate::chain::{ChainClient, ChainError, ChainSettle};
use crate::error::VerifyError;

use super::SchemeHandler;

/// Implicit asset identifier for the `(exact, nockchain:*)` scheme.
/// The Nockchain payload does not carry an explicit asset field —
/// native Nicks are the only payment form — so `requirements.asset`
/// MUST equal this string for the payload to be accepted.
const NOCKCHAIN_NATIVE_ASSET: &str = "NOCK";

#[derive(Debug, Default, Clone, Copy)]
pub struct ExactNockchainHandler;

#[async_trait]
impl SchemeHandler for ExactNockchainHandler {
    fn scheme(&self) -> &'static str {
        "exact"
    }

    fn network_prefix(&self) -> &'static str {
        "nockchain:"
    }

    fn extract_authorization(
        &self,
        payload: &PaymentPayload<Value>,
    ) -> Result<Authorization, VerifyError> {
        let nock: ExactNockchainPayload = serde_json::from_value(payload.payload.clone())
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
        let nock: ExactNockchainPayload = serde_json::from_value(payload.payload.clone())
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

/// Reject `auth.value > requirements.max_amount_required`. Both fields
/// are decimal strings; parse to `u128` to preserve precision (Nicks
/// fit in u64, but the spec leaves the asset semantic open and we
/// never want a silent downcast on the server side). The R1.1 check
/// lives here for `exact`; R1.2's `upto` handler shares the same
/// helper because the comparison direction is identical (`upto` admits
/// any value not exceeding the cap).
pub(super) fn check_max_amount(
    auth: &Authorization,
    req: &PaymentRequirements,
) -> Result<(), VerifyError> {
    let value: u128 = auth.value.parse().map_err(|_| VerifyError::SpecCoded {
        code: "invalid_payload".into(),
        message: format!("authorization.value `{}` is not a u128", auth.value),
    })?;
    let cap: u128 = req
        .max_amount_required
        .parse()
        .map_err(|_| VerifyError::SpecCoded {
            code: "invalid_payload".into(),
            message: format!(
                "requirements.max_amount_required `{}` is not a u128",
                req.max_amount_required,
            ),
        })?;
    if value > cap {
        return Err(VerifyError::OverMaxAmount {
            value: auth.value.clone(),
            max_amount_required: req.max_amount_required.clone(),
        });
    }
    Ok(())
}

/// `requirements.asset` must equal `"NOCK"` for any
/// `(*, nockchain:*)` handler. Shared with the `upto` handler.
pub(super) fn check_asset(req: &PaymentRequirements) -> Result<(), VerifyError> {
    if req.asset.as_str() != NOCKCHAIN_NATIVE_ASSET {
        return Err(VerifyError::AssetMismatch {
            expected: NOCKCHAIN_NATIVE_ASSET.to_string(),
            actual: req.asset.clone(),
        });
    }
    Ok(())
}
