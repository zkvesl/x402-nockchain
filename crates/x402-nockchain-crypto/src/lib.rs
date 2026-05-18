//! Nockchain-specific x402 glue around `vesl-signing`'s canonical
//! Schnorr-over-Cheetah primitives.
//!
//! ## Phase 0 lift (W1-3)
//!
//! The canonical signing API used to live here, in
//! `src/{canonical,schnorr,replay_cache,siwn,math}.rs`. Per
//! `vesl-labs/docs/plans/shared-infrastructure/10-PHASE-0-NOW.md` it
//! moved to a standalone home at `github.com/zkvesl/vesl-wallet` so
//! every Vesl product line (x402, vesl-agent, trust-anchor, oracle
//! services) can depend on the same canonicalization without going
//! through this crate.
//!
//! What stays here is the x402-specific wrapper:
//!
//! - [`sign_message`] — the §5.4.1 `Authorization` Tip5 digest recipe.
//! - [`signer`] — `NockchainSigner`, an `x402_client::Signer` impl.
//! - [`verifier`] — `NockchainVerifier`, payload-level Schnorr check.
//! - [`wire_compat`] — bridges between vesl-signing's
//!   `SchnorrSignatureJson` and the network-neutral `x402-types`
//!   counterpart (the orphan rule prevents `From` impls so they live as
//!   free functions here).
//!
//! Everything else is re-exported from `vesl-signing` so that
//! pre-lift consumers (this crate's siblings — wallet-client,
//! facilitator) keep building unchanged.

pub mod sign_message;
pub mod signer;
pub mod verifier;
pub mod wire_compat;

// === Re-exports preserving the pre-lift API surface ====================

pub use vesl_signing::caip122 as siwn;
pub use vesl_signing::domain as canonical;
pub use vesl_signing::replay_cache;
pub use vesl_signing::schnorr;

pub use vesl_signing::caip122::{
    build_caip122_message, SiwnError, SiwnHeader, SiwnParams, SiwnSigner, VerifiedIdentity,
};
pub use vesl_signing::domain::domain_separators;
pub use vesl_signing::replay_cache::{
    domains as replay_domains, prefixed as prefixed_replay_key, InMemoryReplayCache, ReplayCache,
};
pub use vesl_signing::schnorr::{SchnorrError, SchnorrPrivateKey};

// Curated math access for sibling crates that hand-construct test
// fixtures (golden vectors, scheme-registry tests, wallet-client
// embedded recipe). Goes through the same vesl-signing prelude that
// vesl-core's signing shim uses.
pub mod prelude {
    pub use vesl_signing::prelude::{hash_varlen, Belt, PRIME};
    pub use vesl_signing::schnorr::{CheetahError, CheetahPoint};
}

// x402-specific exports (kept locally — these are NOT lifted to
// vesl-signing because they couple to `x402-types::Authorization`
// and `x402_client::Signer`).
pub use sign_message::{
    base58_to_belts, pkh_belts_to_base58, pkh_from_pubkey_bytes, x402_sign_message_belts,
    x402_sign_message_digest, SignMessageError, X402_DOMAIN_SEPARATOR,
};
pub use signer::NockchainSigner;
pub use verifier::{NockchainVerifier, VerifyError};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctest;
