//! `NockchainVerifier` — verifies a `PaymentPayload<Value>` under the
//! `(exact, nockchain:*)` scheme.
//!
//! Envelope checks (version, scheme, network, `to == payTo`) stay in the
//! facilitator's envelope-validation pass; this module focuses on the
//! cryptographic verification: spec-§5.4.1 Tip5 sponge digest of the
//! authorization fields, Schnorr check against the carried
//! `(pubkey, chal, sig)`.
//!
//! ## Phase 0 lift
//!
//! `decode_signature`, `schnorr_verify`, and the `SchnorrError` /
//! `CheetahError` types now live in `vesl-signing`. The wire type carried
//! on `ExactNockchainPayload.signature` is x402-types', so we convert it
//! to vesl-signing's form via [`crate::wire_compat::x402_to_vesl`] before
//! handing it to the verifier.

use thiserror::Error;
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{PaymentPayload, PaymentRequirements};

use vesl_signing::schnorr::{decode_signature, schnorr_verify, CheetahPoint, SchnorrError};

use crate::sign_message::{pkh_from_pubkey_bytes, x402_sign_message_digest, SignMessageError};
use crate::wire_compat::x402_to_vesl;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("payload JSON does not match ExactNockchainPayload: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("signature decode: {0}")]
    SignatureDecode(SchnorrError),
    #[error("signature does not verify: {0}")]
    SignatureInvalid(SchnorrError),
    #[error("failed to compute sign_message digest: {0}")]
    SignMessage(#[from] SignMessageError),
    /// `Tip5(signature.pubkey)` does not equal `authorization.from`.
    /// Per `06-facilitator.md §6.4` check 16 — prevents a valid
    /// signature under a different PKH than the authorization claims.
    #[error("pubkey binding failed: Tip5(pubkey) != authorization.from")]
    PubkeyBinding,
}

/// Stateless verifier for `(exact, nockchain:*)` payloads.
#[derive(Clone, Copy, Debug, Default)]
pub struct NockchainVerifier;

impl NockchainVerifier {
    pub fn new() -> Self {
        Self
    }

    /// Verify that `payload` carries a valid Schnorr signature over the
    /// spec §5.4.1 Tip5 digest of the authorization. Envelope-level
    /// checks (`scheme`, `network`, `payTo` match) are expected to have
    /// already passed.
    pub fn verify_payment<P: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        payload: &PaymentPayload<P>,
        _requirements: &PaymentRequirements,
    ) -> Result<(), VerifyError> {
        let nock: ExactNockchainPayload =
            match serde_json::to_value(&payload.payload).and_then(serde_json::from_value) {
                Ok(v) => v,
                Err(e) => return Err(VerifyError::Decode(e)),
            };

        let vesl_sig = x402_to_vesl(&nock.signature);
        let (pk, chal, sig) = decode_signature(&vesl_sig).map_err(VerifyError::SignatureDecode)?;

        // §6.4 check 16: Tip5(pubkey) MUST equal authorization.from.
        let pk_bytes = pk
            .to_bytes()
            .map_err(|e| VerifyError::SignatureDecode(SchnorrError::Curve(e)))?;
        let expected_pkh = pkh_from_pubkey_bytes(&pk_bytes);
        if expected_pkh != nock.authorization.from {
            return Err(VerifyError::PubkeyBinding);
        }

        let digest = x402_sign_message_digest(&nock.authorization)?;

        schnorr_verify(&pk, &digest, &chal, &sig).map_err(VerifyError::SignatureInvalid)
    }
}

// Internal: keeps the CheetahPoint type used for `pk_bytes` accessible.
#[allow(dead_code)]
fn _unused(_: &CheetahPoint) {}
