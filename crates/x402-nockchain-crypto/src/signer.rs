//! `NockchainSigner` — implements `x402_client::Signer` on top of a
//! `vesl_signing::SchnorrPrivateKey`.
//!
//! Signs the 5-Belt Tip5 digest defined by
//! [`crate::sign_message::x402_sign_message_digest`], matching
//! `docs/specs-snapshot/05-payload.md §5.4.1` exactly.
//!
//! [`Signer::from_identifier`] returns the payer's PKH (= base58 of
//! Tip5(pubkey_bytes)), **not** the full Cheetah pubkey — per
//! `05-payload.md §5.3.2`, `Authorization.from` is a PKH. The full
//! pubkey is carried separately on [`SchnorrSignatureJson::pubkey`] so
//! the verifier has everything needed for both (a) Tip5(pubkey) == from
//! binding and (b) the Schnorr check.
//!
//! ## Phase 0 lift
//!
//! `schnorr_sign`, `encode_signature`, and `CheetahPoint` now live in
//! `vesl-signing`. `vesl-signing` owns its own `SchnorrSignatureJson`
//! wire type; the `x402_client::Signer` trait expects the network-neutral
//! [`x402_types::payment::SchnorrSignatureJson`] form, so we convert at
//! the trait boundary via [`crate::wire_compat::vesl_to_x402`].

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use x402_client::Signer;
use x402_types::payment::{Authorization, PaymentRequirements, SchnorrSignatureJson};

use vesl_signing::schnorr::{encode_signature, schnorr_sign, CheetahPoint, SchnorrPrivateKey};

use crate::sign_message::{pkh_from_pubkey_bytes, x402_sign_message_digest};
use crate::wire_compat::vesl_to_x402;

/// Signer for the `(exact, nockchain:*)` scheme.
pub struct NockchainSigner {
    sk: SchnorrPrivateKey,
    pk: CheetahPoint,
    pk_b58: String,
    pkh_b58: String,
}

impl NockchainSigner {
    pub fn new(sk: SchnorrPrivateKey) -> Result<Self> {
        let pk = sk.public_key();
        let pk_b58 = pk
            .into_base58()
            .map_err(|e| anyhow!("encode Cheetah pubkey: {e}"))?;
        let pk_bytes = pk
            .to_bytes()
            .map_err(|e| anyhow!("serialize Cheetah pubkey: {e}"))?;
        let pkh_b58 = pkh_from_pubkey_bytes(&pk_bytes);
        Ok(Self {
            sk,
            pk,
            pk_b58,
            pkh_b58,
        })
    }

    pub fn public_key_point(&self) -> &CheetahPoint {
        &self.pk
    }
    pub fn public_key_base58(&self) -> &str {
        &self.pk_b58
    }
    pub fn pkh_base58(&self) -> &str {
        &self.pkh_b58
    }
}

#[async_trait]
impl Signer for NockchainSigner {
    async fn sign_authorization(
        &self,
        auth: &Authorization,
        _requirements: &PaymentRequirements,
    ) -> Result<SchnorrSignatureJson> {
        // The §5.4.1 digest is over `auth` alone; `requirements` is the
        // signer-side policy seam consumed by wrappers (per
        // `x402_client::policy::PolicyEnforcedSigner`).
        let digest = x402_sign_message_digest(auth)
            .map_err(|e| anyhow!("compute sign_message digest: {e}"))?;
        let (chal, sig) =
            schnorr_sign(&self.sk, &digest).map_err(|e| anyhow!("schnorr sign: {e}"))?;
        let vesl_sig = encode_signature(&self.pk, &chal, &sig)
            .map_err(|e| anyhow!("encode signature: {e}"))?;
        Ok(vesl_to_x402(&vesl_sig))
    }

    fn from_identifier(&self) -> String {
        self.pkh_b58.clone()
    }
}
