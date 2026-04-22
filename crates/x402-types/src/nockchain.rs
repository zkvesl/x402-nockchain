//! Nockchain-specific payload variants (`ExactNockchainPayload`, etc.).
//!
//! Gated behind the `nockchain` cargo feature. A hypothetical future EVM/SVM
//! adapter would provide its own sibling module under its own feature gate
//! without touching `payment.rs`.
