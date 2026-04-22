//! Shared types for the x402 Rust ecosystem.
//!
//! Network-neutral core. Nockchain-specific payload variants live behind the
//! `nockchain` feature flag (enabled by default in this workspace).
//!
//! Module map:
//! - [`bazaar`] — Bazaar discovery-extension types (populated at M0).
//! - [`payment`] — `PaymentRequirements`, `PaymentPayload`, `Authorization`
//!   (populated at M1; see `vesl-agent/docs/plans/phase-1-types-frozen.md`).
//! - [`siwn`] — Sign-In-With-Nockchain / CAIP-122 types (populated at M3).
//! - [`nockchain`] — Nockchain-specific payload variants, `nockchain` feature
//!   only (populated at M1 alongside `payment`).

pub mod bazaar;
pub mod payment;
pub mod siwn;

#[cfg(feature = "nockchain")]
pub mod nockchain;

pub use bazaar::*;
