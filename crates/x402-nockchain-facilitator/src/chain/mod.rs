//! Chain-submission abstraction for `/settle`.
//!
//! `/settle` routes a verified x402 `SettleRequest` through a [`ChainClient`]
//! implementation. Two impls ship:
//!
//! - [`StubChainClient`]: synthesises a deterministic `tx_id` and returns
//!   `Broadcast` without touching any chain. Used by the Phase-2/3
//!   regression tests and by `examples/e2e_demo` in stub mode.
//! - [`GrpcChainClient`]: wraps `nockchain_client_rs::ChainClient` and
//!   submits real `RawTx`s against a fakenet (or any Nockchain public gRPC
//!   endpoint), polling for acceptance with the canonical hull-rag
//!   poll-interval + deadline semantics.
//!
//! The two-impl layout means the default test path stays fast and
//! deterministic, and the real-chain path ships as an opt-in for
//! fakenet/testnet runs (`examples/demo.sh`).
//!
//! The Authorization→RawTx construction inside [`GrpcChainClient::settle`]
//! still depends on the canonical-hashing work flagged in ADR-0007: the
//! signature our Phase-3 signer produces is over sorted-JSON bytes, while
//! the chain verifier hashes a noun encoding. Surfacing that gap is one of
//! Phase 4's real deliverables — [`ChainError::TxConstructionUnavailable`]
//! is the explicit carrier until the canonicalisation is aligned.

pub mod grpc;
pub mod stub;

use async_trait::async_trait;
use x402_types::facilitator::SettleRequest;

pub use grpc::GrpcChainClient;
pub use stub::StubChainClient;

/// Outcome of a settlement submission.
#[derive(Debug, Clone)]
pub struct ChainSettle {
    /// Base58-encoded chain transaction id.
    pub tx_id: String,
    /// Block height of inclusion, if known at response time.
    pub block_height: Option<u64>,
    /// Lifecycle marker — maps to `SettleResponse.transaction.status`.
    pub status: ChainStatus,
}

/// Lifecycle state of a submitted transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStatus {
    /// Submitted to the node, no acceptance signal yet.
    Broadcast,
    /// Polling timed out before block inclusion. Per `bazaar.md`, this
    /// surfaces as `EXTENSION-RESPONSES: {"bazaar":{"status":"processing"}}`.
    Processing,
    /// Chain confirmed block inclusion.
    Accepted,
}

impl ChainStatus {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Broadcast => "broadcast",
            Self::Processing => "processing",
            Self::Accepted => "accepted",
        }
    }
}

/// Errors returned by a [`ChainClient`].
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    /// Network / gRPC transport failure.
    #[error("chain transport error: {0}")]
    Transport(String),
    /// The node rejected the transaction outright (malformed, bad signature,
    /// insufficient funds, etc).
    #[error("chain rejected transaction: {0}")]
    Rejected(String),
    /// Authorization → RawTx construction path isn't available in this
    /// build. Carrier for the canonical-hashing gap per ADR-0007.
    #[error("tx construction unavailable: {0}")]
    TxConstructionUnavailable(String),
}

/// Abstraction over "submit a verified x402 settlement to the chain".
///
/// The trait takes a `SettleRequest` directly (rather than a constructed
/// `RawTx`) so implementations can choose their own construction path —
/// the stub skips construction entirely, and the gRPC impl owns both the
/// translation and the submission loop.
#[async_trait]
pub trait ChainClient: Send + Sync {
    async fn settle(&self, req: &SettleRequest) -> Result<ChainSettle, ChainError>;
}
