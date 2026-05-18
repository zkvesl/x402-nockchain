//! Nockchain-specific payload variants for [`crate::payment::PaymentPayload`].
//!
//! Gated behind the `nockchain` cargo feature. A hypothetical future EVM/SVM
//! adapter would provide its own sibling module under its own feature gate
//! without touching `payment.rs`.
//!
//! Shape per PR #102 `specs/x402/05-payment-payload.md §5.3`, pinned under
//! `docs/specs-snapshot/`.

use serde::{Deserialize, Serialize};

use crate::payment::{Authorization, SchnorrSignatureJson};

/// The `payload` field of a `PaymentPayload` when the selected scheme /
/// network pair is `(exact, nockchain:*)`. Pairs the signer's Schnorr
/// signature with the signed [`Authorization`] object and the fully-
/// signed [`SignedRawTx`] the wallet assembled.
///
/// Use with the generic [`crate::payment::PaymentPayload<P>`]:
///
/// ```ignore
/// use x402_types::payment::PaymentPayload;
/// use x402_types::nockchain::ExactNockchainPayload;
///
/// let typed: PaymentPayload<ExactNockchainPayload> =
///     serde_json::from_str(wire_json).unwrap();
/// ```
///
/// `signed_raw_tx` is optional during the Phase-4 transition: payloads
/// produced before path 2B shipped (ADR-0010) omit it, and the Phase-3
/// roundtrip tests continue to exercise envelope-only verification.
/// Real fakenet `/settle` requires the field to be present.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExactNockchainPayload {
    /// Payer's Schnorr public key and the signature over
    /// [`ExactNockchainPayload::authorization`] (per §5.4.1).
    pub signature: SchnorrSignatureJson,
    /// The signed authorization (recipient, amount, fee, notes, …).
    pub authorization: Authorization,
    /// Wallet-assembled signed `RawTx` the facilitator submits. JSON
    /// surrogate per ADR-0010 + `docs/wallet-integration.md §7`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_raw_tx: Option<SignedRawTx>,
}

/// The `payload` field of a `PaymentPayload` when the selected scheme /
/// network pair is `(upto, nockchain:*)`. Structurally identical to
/// [`ExactNockchainPayload`] — `upto` is a verifier-side semantic
/// (accepts any `auth.value` not exceeding `requirements.max_amount_required`)
/// and the spec does not pin a different wire shape.
///
/// The dedicated type alias exists so per-scheme handlers can take a
/// typed payload without dropping back into `serde_json::Value`. If a
/// future spec revision diverges the shapes, this becomes a real
/// struct without churning the call sites that already name this type.
pub type UptoNockchainPayload = ExactNockchainPayload;

// ---------------------------------------------------------------------------
// Signed RawTx JSON surrogate — per ADR-0010 path 2B
// ---------------------------------------------------------------------------

/// Top-level JSON surrogate of a chain-ready, fully-signed `RawTx`.
///
/// Mirrors `nockchain_types::tx_engine::v1::RawTx` at the fields a
/// payment auditor cares about (version, tx_id, spends with their
/// recipient/amount/fee). Deep sub-trees that don't have a natural JSON
/// form — lock-merkle proofs, SpendCondition internal scripts, NoteData
/// ZMaps — ride as base64'd jammed-noun bytes in `*_noun` fields until
/// upstream `nockchain-types` grows serde derives.
///
/// The conversion between this surrogate and the noun-typed chain
/// representation lives in `x402-nockchain-crypto` (outside this crate
/// so x402-types remains network-neutral at the dep level).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedRawTx {
    /// Protocol version. Currently only `"v1"` is supported.
    pub version: String,
    /// base58-encoded Tip5 hash of the serialized transaction.
    pub tx_id: String,
    /// Ordered per-note spends.
    pub spends: Vec<SignedSpendEntry>,
}

/// One `(Name, Spend)` pair in [`SignedRawTx::spends`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedSpendEntry {
    pub name: SignedNoteName,
    pub spend: SignedSpend,
}

/// Two-hash Nockchain note name as base58 strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedNoteName {
    pub first: String,
    pub last: String,
}

/// Mirrors `Spend::Witness(Spend1 { witness, seeds, fee })`. Legacy
/// (v0) `Spend::Legacy` is not supported for x402 settlement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedSpend {
    pub witness: SignedWitness,
    pub seeds: Vec<SignedSeed>,
    /// Fee in nicks as decimal string.
    pub fee: String,
}

/// Mirrors `Witness` with hybrid encoding. `lock_merkle_proof_noun` is
/// opaque — base64 of the jammed noun — because the underlying
/// `LockMerkleProof` / `SpendCondition` types don't have a natural JSON
/// form. `pkh_signature` stays typed so payment auditors can read who
/// signed what.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedWitness {
    /// base64 of the jammed `LockMerkleProof`. Opaque by design.
    pub lock_merkle_proof_noun: String,
    /// One entry per signer the lock requires.
    pub pkh_signature: Vec<SignedPkhSignatureEntry>,
    /// Optional HAX preimages. Usually empty for simple PKH spends.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hax: Vec<SignedHaxPreimage>,
    /// Always `0` for v1 spends. Present in the wire form so the
    /// surrogate survives a future v2 widening without a schema break.
    #[serde(default)]
    pub tim: u64,
}

/// Mirrors `PkhSignatureEntry`. All three fields are base58-typed and
/// directly human-readable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedPkhSignatureEntry {
    /// base58 PKH of the spender. MUST equal `Tip5(pubkey)`.
    pub hash: String,
    /// Full Cheetah pubkey, base58 (97-byte form).
    pub pubkey: String,
    /// Schnorr signature over the chain's `sig_hash` for this spend.
    pub signature: SignedSchnorrAtoms,
}

/// The `(chal, sig)` pair of a Schnorr-over-Cheetah signature. Each
/// array is exactly 8 Belt elements as decimal strings — same wire
/// shape as [`crate::payment::SchnorrPair`] but kept separate so
/// downstream changes to the x402 envelope signature format don't
/// cascade into the on-chain surrogate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedSchnorrAtoms {
    pub chal: [String; 8],
    pub sig: [String; 8],
}

/// Mirrors `HaxPreimage`. `value_noun` is base64 of the jammed value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedHaxPreimage {
    pub hash: String,
    pub value_noun: String,
}

/// Mirrors `Seed` with hybrid encoding. Payment-critical fields
/// (`lock_root`, `gift`, `parent_hash`) are typed JSON; `output_source`
/// and `note_data` are opaque noun bags because their internal shapes
/// aren't load-bearing for a payment auditor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedSeed {
    /// base64 jammed `Option<Source>`. `None` on the wire means the
    /// seed has no explicit output source; most x402 seeds omit this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_source_noun: Option<String>,
    /// base58 hash of the recipient's lock-tree root.
    pub lock_root: String,
    /// base64 jammed `NoteData` (the key/value entries carried with the
    /// note). `None` means no app-specific data attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_data_noun: Option<String>,
    /// Output amount in nicks as decimal string.
    pub gift: String,
    /// base58 hash of the parent note (the UTXO being spent to create
    /// this output).
    pub parent_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payment::{SchnorrPair, SchnorrSignatureJson};

    fn zero_sig_atoms() -> SignedSchnorrAtoms {
        SignedSchnorrAtoms {
            chal: ["0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(),
                   "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string()],
            sig: ["0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(),
                  "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string()],
        }
    }

    fn fixture_signed_raw_tx() -> SignedRawTx {
        SignedRawTx {
            version: "v1".into(),
            tx_id: "5Ht7Rk3qX9abcfg".into(),
            spends: vec![SignedSpendEntry {
                name: SignedNoteName {
                    first: "3J98t1WpEZ73CN".into(),
                    last: "2Pq3rS4tU5vW6x".into(),
                },
                spend: SignedSpend {
                    witness: SignedWitness {
                        lock_merkle_proof_noun: "opaqueBase64==".into(),
                        pkh_signature: vec![SignedPkhSignatureEntry {
                            hash: "4Ab2c3D4e5F6g7".into(),
                            pubkey: "3WhcwvTgGQDxohSW".into(),
                            signature: zero_sig_atoms(),
                        }],
                        hax: vec![],
                        tim: 0,
                    },
                    seeds: vec![SignedSeed {
                        output_source_noun: None,
                        lock_root: "1A1zP1eP5QGefi".into(),
                        note_data_noun: None,
                        gift: "65536".into(),
                        parent_hash: "4Ab2c3D4e5F6g7".into(),
                    }],
                    fee: "10".into(),
                },
            }],
        }
    }

    #[test]
    fn signed_raw_tx_roundtrips_through_json() {
        let orig = fixture_signed_raw_tx();
        let json = serde_json::to_string(&orig).unwrap();
        let back: SignedRawTx = serde_json::from_str(&json).unwrap();
        assert_eq!(orig, back);
    }

    #[test]
    fn payload_with_signed_raw_tx_roundtrips() {
        let signed = fixture_signed_raw_tx();
        let payload = ExactNockchainPayload {
            signature: SchnorrSignatureJson {
                pubkey: "stub-pk".into(),
                schnorr: SchnorrPair {
                    chal: zero_sig_atoms().chal,
                    sig: zero_sig_atoms().sig,
                },
            },
            authorization: Authorization {
                from: "from-pkh".into(),
                to: "to-pkh".into(),
                value: "1".into(),
                fee: "0".into(),
                nonce: "nonce".into(),
                valid_after: 0,
                valid_before: 100,
                notes: vec![],
                change_address: "from-pkh".into(),
            },
            signed_raw_tx: Some(signed.clone()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: ExactNockchainPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, back);
        assert_eq!(back.signed_raw_tx.unwrap(), signed);
    }

    #[test]
    fn payload_without_signed_raw_tx_still_parses() {
        // Phase-3 payloads must continue to deserialize — `signedRawTx`
        // is optional for backwards compat during the path-2B rollout.
        let phase3_json = serde_json::json!({
            "signature": { "pubkey": "stub-pk", "schnorr": { "chal": ["0","0","0","0","0","0","0","0"], "sig": ["0","0","0","0","0","0","0","0"] } },
            "authorization": {
                "from": "from-pkh", "to": "to-pkh", "value": "1", "fee": "0",
                "nonce": "nonce", "validAfter": 0, "validBefore": 100,
                "notes": [], "changeAddress": "from-pkh"
            }
        });
        let decoded: ExactNockchainPayload = serde_json::from_value(phase3_json).unwrap();
        assert!(decoded.signed_raw_tx.is_none());
    }
}
