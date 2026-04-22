//! Schnorr-over-Cheetah signing/verification, Tip5 domain separators, and
//! SIWN (CAIP-122) for Nockchain.
//!
//! Scaffold at M0; implemented at M3 per
//! `vesl-agent/docs/plans/phase-3-real-signing-siwn.md`. Will wrap primitives
//! from `nockchain/crates/nockchain-math` (git dep pinned by M4) and implement
//! [`x402_client::Signer`].
