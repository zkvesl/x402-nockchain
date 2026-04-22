//! Shared types for the x402 Rust ecosystem.
//!
//! Network-neutral core. Nockchain-specific payload variants live behind the
//! `nockchain` feature flag (enabled by default in this workspace).
//!
//! Module map:
//! - [`bazaar`] — Bazaar discovery-extension types.
//! - [`payment`] — `PaymentRequirements`, `PaymentPayload`, `Authorization`.
//! - [`siwn`] — Sign-In-With-Nockchain / CAIP-122 types.
//! - [`nockchain`] — Nockchain-specific payload variants, `nockchain` feature only.

pub mod bazaar;
pub mod payment;
pub mod siwn;

#[cfg(feature = "nockchain")]
pub mod nockchain;

pub use bazaar::*;
