//! Per-`(scheme, network)` verification + settlement dispatch.
//!
//! Pre-R1.2, `verify_envelope` and `settle` hardcoded the
//! `(exact, nockchain:*)` branch and silently no-op'd everything else.
//! R1.2 lifts that into a `SchemeHandler` trait registered on
//! [`SchemeHandlerRegistry`]; unknown `(scheme, network)` pairs now
//! reject with [`VerifyError::UnknownScheme`] (a *correctness*
//! improvement — see ADR-0020 for the behavior-change note).
//!
//! Two handlers ship in this commit:
//!
//! - [`exact_nockchain::ExactNockchainHandler`] — preserves R1.1's
//!   `(exact, nockchain:*)` semantics byte-identically.
//! - [`upto_nockchain::UptoNockchainHandler`] — registers the
//!   `(upto, nockchain:*)` variant; same wire shape, same accept
//!   condition (`auth.value <= max`), but the *expectation* differs
//!   per the spec's `upto` semantics. See ADR-0020.

pub mod exact_nockchain;
pub mod upto_nockchain;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use x402_types::facilitator::SettleRequest;
use x402_types::payment::{Authorization, PaymentPayload, PaymentRequirements};

use crate::chain::{ChainClient, ChainError, ChainSettle};
use crate::error::VerifyError;

pub use exact_nockchain::ExactNockchainHandler;
pub use upto_nockchain::UptoNockchainHandler;

/// Per-`(scheme, network)` handler. Implementations encapsulate the
/// scheme-specific payload-decode + validation + chain-submit logic.
///
/// The pre-dispatch path in [`crate::verify_envelope`] handles
/// scheme-independent checks (envelope version/scheme/network match,
/// time window, nonce replay) before calling [`SchemeHandler::verify`].
/// The handler's `verify` is responsible for everything else —
/// payload decode, recipient binding, signature verification,
/// max-amount enforcement, asset matching.
///
/// `settle` is the chain-submit step. Today every handler delegates to
/// the configured [`ChainClient`]; the trait surface allows a future
/// non-chain settlement path (e.g. a credit-tracking handler) to
/// override.
#[async_trait]
pub trait SchemeHandler: Send + Sync {
    /// Stable string label this handler claims for `PaymentRequirements.scheme`.
    fn scheme(&self) -> &'static str;

    /// Network prefix this handler claims (e.g. `"nockchain:"`). The
    /// registry lookup matches via `network.starts_with(prefix)`.
    fn network_prefix(&self) -> &'static str;

    /// Decode the scheme-specific payload and return the universal
    /// [`Authorization`] view. Used by the pre-dispatch path's
    /// time-window + replay checks. Implementations MAY assume the
    /// envelope `(scheme, network)` already matches.
    fn extract_authorization(
        &self,
        payload: &PaymentPayload<Value>,
    ) -> Result<Authorization, VerifyError>;

    /// Apply per-scheme verification: payload decode, `to == payTo`,
    /// signature, max-amount cap, asset match. Time-window and replay
    /// are handled by the pre-dispatch caller.
    async fn verify(
        &self,
        payload: &PaymentPayload<Value>,
        requirements: &PaymentRequirements,
    ) -> Result<(), VerifyError>;

    /// Submit the settled transaction. Default implementations delegate
    /// to the supplied [`ChainClient`].
    async fn settle(
        &self,
        chain: &dyn ChainClient,
        request: &SettleRequest,
    ) -> Result<ChainSettle, ChainError>;
}

/// Ordered list of registered handlers. Lookup is first-match by
/// `(scheme, network)`; ties go to the first registered. Cheap to
/// clone — the inner `Vec` holds `Arc`-shared trait objects.
#[derive(Clone, Default)]
pub struct SchemeHandlerRegistry {
    handlers: Vec<Arc<dyn SchemeHandler>>,
}

impl SchemeHandlerRegistry {
    /// Empty registry. Use [`SchemeHandlerRegistry::with_default_handlers`]
    /// for the production set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler. Order is significant — first matching
    /// `(scheme, network)` wins.
    pub fn register(&mut self, handler: Arc<dyn SchemeHandler>) {
        self.handlers.push(handler);
    }

    /// Find the handler that claims `(scheme, network)`, or `None` if
    /// no handler matches. Match is exact on `scheme` and prefix on
    /// `network`.
    pub fn handler_for(&self, scheme: &str, network: &str) -> Option<Arc<dyn SchemeHandler>> {
        self.handlers
            .iter()
            .find(|h| h.scheme() == scheme && network.starts_with(h.network_prefix()))
            .cloned()
    }

    /// Registry pre-populated with the in-tree handlers shipped at the
    /// R1.2 close: `(exact, nockchain:*)` and `(upto, nockchain:*)`.
    pub fn with_default_handlers() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(ExactNockchainHandler::default()));
        reg.register(Arc::new(UptoNockchainHandler::default()));
        reg
    }

    /// Number of registered handlers. Useful for tests.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// True iff no handlers are registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}
