//! Caller-side policy seam for [`Signer`][crate::Signer].
//!
//! [`PolicyEnforcedSigner`] wraps any inner [`Signer`] with a synchronous
//! closure that inspects the [`PaymentRequirements`] before delegating.
//! When the closure returns [`Err(PolicyDenied)`][PolicyDenied], the
//! adapter short-circuits and the inner signer is never called — the
//! authorization is therefore unsigned, and no facilitator can settle
//! against it.
//!
//! This is the client-side complement of the server-side `verify_envelope`
//! checks landed in R1.1: the verifier rejects authorizations whose
//! requirements violate spec, but a signer that enforces its own bounds
//! refuses to produce a settle-able envelope in the first place. Both
//! ends answer the same enforceability question — what can be inferred
//! from `(Authorization, PaymentRequirements)` alone — at the two ends
//! of the wire.
//!
//! ## Example
//!
//! ```ignore
//! use x402_client::{policy::PolicyEnforcedSigner, NockchainSigner};
//!
//! let inner = NockchainSigner::new(sk)?;
//! let max_value = 1_000_000u128;
//! let signer = PolicyEnforcedSigner::new(inner, move |req| {
//!     let cap: u128 = req.max_amount_required.parse()
//!         .map_err(|_| PolicyDenied::new("max_amount_required is not a u128"))?;
//!     if cap > max_value {
//!         return Err(PolicyDenied::new(format!(
//!             "operator policy caps max_amount_required at {max_value}, got {cap}"
//!         )));
//!     }
//!     Ok(())
//! });
//! ```

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use x402_types::payment::{Authorization, PaymentRequirements, SchnorrSignatureJson};

use crate::Signer;

/// Reason a [`PolicyEnforcedSigner`] refused to produce a signature.
///
/// Carries a human-readable string for inclusion in error chains. Wrapping
/// signers MAY map this to richer typed errors at their boundary; the
/// adapter itself surfaces it via [`anyhow::Error`].
#[derive(Debug, Clone, thiserror::Error)]
#[error("policy denied: {reason}")]
pub struct PolicyDenied {
    pub reason: String,
}

impl PolicyDenied {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }
}

/// Type alias for the policy closure carried on a [`PolicyEnforcedSigner`].
///
/// Implementations are pure synchronous functions of
/// [`PaymentRequirements`]; if your policy requires async I/O (e.g. a
/// remote allowlist lookup), pre-compute the decision and embed it in
/// the closure's environment.
pub type PolicyFn = Arc<dyn Fn(&PaymentRequirements) -> Result<(), PolicyDenied> + Send + Sync>;

/// `Signer` adapter that runs a policy closure over [`PaymentRequirements`]
/// before delegating to an inner signer.
///
/// Cheap to clone — the inner signer is held by `Arc` and the policy is
/// already an `Arc<dyn Fn>`.
#[derive(Clone)]
pub struct PolicyEnforcedSigner<S: Signer> {
    inner: Arc<S>,
    policy: PolicyFn,
}

impl<S: Signer> PolicyEnforcedSigner<S> {
    /// Construct a new adapter from an inner signer and a policy closure.
    pub fn new<F>(inner: S, policy: F) -> Self
    where
        F: Fn(&PaymentRequirements) -> Result<(), PolicyDenied> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(inner),
            policy: Arc::new(policy),
        }
    }

    /// Construct an adapter from a pre-`Arc`'d signer + policy. Useful
    /// when the inner signer is shared across multiple adapters with
    /// different policies (e.g. one per resource class).
    pub fn from_arc(inner: Arc<S>, policy: PolicyFn) -> Self {
        Self { inner, policy }
    }
}

#[async_trait]
impl<S: Signer> Signer for PolicyEnforcedSigner<S> {
    async fn sign_authorization(
        &self,
        auth: &Authorization,
        requirements: &PaymentRequirements,
    ) -> Result<SchnorrSignatureJson> {
        (self.policy)(requirements).map_err(|denied| anyhow!(denied))?;
        self.inner.sign_authorization(auth, requirements).await
    }

    fn from_identifier(&self) -> String {
        self.inner.from_identifier()
    }
}
