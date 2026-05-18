//! Shared types for the x402 Rust ecosystem.
//!
//! Network-neutral core. Nockchain-specific payload variants live behind the
//! `nockchain` feature flag (enabled by default in this workspace).
//!
//! Module map:
//! - [`bazaar`] — Bazaar discovery-extension types.
//! - [`payment`] — `PaymentRequirements`, `PaymentPayload`, `Authorization`.
//! - [`facilitator`] — `/verify` / `/settle` request + response types.
//! - [`siwn`] — Sign-In-With-Nockchain / CAIP-122 types.
//! - [`nockchain`] — Nockchain-specific payload variants, `nockchain` feature only.

pub mod bazaar;
pub mod facilitator;
pub mod payment;
pub mod siwn;

#[cfg(feature = "nockchain")]
pub mod nockchain;

pub use bazaar::*;
pub use facilitator::*;
pub use payment::*;
pub use siwn::*;

#[cfg(feature = "nockchain")]
pub use nockchain::*;

// Doctest the README examples (per ADR-0014's note about consumer-example rot).
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctest;
