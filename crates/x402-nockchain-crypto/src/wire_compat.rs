//! Bridge between vesl-signing's `SchnorrSignatureJson` and x402-types'
//! `SchnorrSignatureJson`.
//!
//! Both crates own a structurally-identical wire type — the duplication
//! is intentional:
//!
//! - `vesl-signing`: canonical home of Schnorr-over-Cheetah; defines the
//!   wire type alongside `schnorr_sign` / `encode_signature` /
//!   `decode_signature`. Self-contained, no x402 dep.
//! - `x402-types`: the network-neutral x402 protocol crate, designed for
//!   eventual crates.io publish without a Cheetah-math dep. Keeps its own
//!   `SchnorrSignatureJson` so a consumer can model x402 payloads
//!   without depending on Vesl crypto.
//!
//! The orphan rule prevents `From` impls in this crate (neither type is
//! local), so we expose the conversion as free functions. Field copies
//! are cheap — both structs are identical 8-string arrays plus a base58
//! pubkey.

use vesl_signing::schnorr::{
    SchnorrPair as VeslPair, SchnorrSignatureJson as VeslSig,
};
use x402_types::payment::{
    SchnorrPair as X402Pair, SchnorrSignatureJson as X402Sig,
};

/// Convert a vesl-signing wire signature into the x402-types form.
pub fn vesl_to_x402(s: &VeslSig) -> X402Sig {
    X402Sig {
        pubkey: s.pubkey.clone(),
        schnorr: X402Pair {
            chal: s.schnorr.chal.clone(),
            sig: s.schnorr.sig.clone(),
        },
    }
}

/// Convert an x402-types wire signature into the vesl-signing form.
pub fn x402_to_vesl(s: &X402Sig) -> VeslSig {
    VeslSig {
        pubkey: s.pubkey.clone(),
        schnorr: VeslPair {
            chal: s.schnorr.chal.clone(),
            sig: s.schnorr.sig.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_x402() -> X402Sig {
        X402Sig {
            pubkey: "abc123".into(),
            schnorr: X402Pair {
                chal: std::array::from_fn(|i| (i as u32 * 7).to_string()),
                sig: std::array::from_fn(|i| (i as u32 * 11).to_string()),
            },
        }
    }

    #[test]
    fn roundtrip_x402_vesl_x402() {
        let original = sample_x402();
        let vesl = x402_to_vesl(&original);
        let back = vesl_to_x402(&vesl);
        assert_eq!(back.pubkey, original.pubkey);
        assert_eq!(back.schnorr.chal, original.schnorr.chal);
        assert_eq!(back.schnorr.sig, original.schnorr.sig);
    }
}
