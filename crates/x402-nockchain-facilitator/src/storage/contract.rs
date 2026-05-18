//! `CatalogStore` trait — the upsert/list contract every catalog backend
//! satisfies.
//!
//! Two impls ship with this crate: [`super::sqlite::SqliteCatalogStore`]
//! (the default; promoted from the inline functions through Phase 4) and
//! [`super::memory::InMemoryCatalogStore`] (zero-dep, used by the tests
//! and demos that don't want to spin a SQLite pool).
//!
//! Future backends sketched in TODOs below — the trait shape was distilled
//! from the real Phases 2-4 SQLite queries, not designed top-down (per the
//! Phase 5A risk note about "trait extraction churn").
//!
//! Both built-in impls share a generic contract test suite at
//! `tests/catalog_store_contract.rs` so a third-party impl can prove
//! compatibility by running the same suite.

use anyhow::Result;
use async_trait::async_trait;
use x402_types::bazaar::{DiscoveryResource, ListDiscoveryResourcesParams};

/// Catalog storage contract.
///
/// Behavioural invariants the contract suite enforces:
///
/// - **Upsert is keyed on `(resource, kind)`.** Re-upserting the same pair
///   replaces `accepts`, `metadata`, `last_updated`, and `x402_version`.
/// - **`list` honours `kind`, `network`, `scheme`, `limit`, `offset`.** Each
///   filter narrows the result; `total` reflects the post-filter count
///   (i.e., it does NOT count rows the filters excluded).
/// - **`network`/`scheme` filters match if ANY entry of `accepts` matches.**
///   String equality on the field; missing field → no match.
/// - **Default `limit` is the impl's choice but `list` MUST tolerate
///   `limit=None`.** The HTTP layer applies its own default before reaching
///   the trait.
/// - **Order is insertion order.** Stable across re-listings between
///   upserts.
///
/// Errors are bubbled through `anyhow::Error`; the facilitator's
/// [`crate::error::AppError`] turns them into `500 Internal Server Error`.
#[async_trait]
pub trait CatalogStore: Send + Sync {
    /// Insert a new resource or replace the existing entry keyed on
    /// `(resource.resource, resource.kind)`.
    async fn upsert(&self, resource: &DiscoveryResource) -> Result<()>;

    /// List catalog entries matching `params`. Returns `(items, total)`
    /// where `total` is the full match count (pre-`limit`/-`offset`).
    async fn list(
        &self,
        params: &ListDiscoveryResourcesParams,
    ) -> Result<(Vec<DiscoveryResource>, u32)>;
}

// ---------------------------------------------------------------------------
// Future-backend skeletons — written here so the next implementer can find
// them without grepping. Both compile-time gated; neither is wired into
// `AppState` today.
// ---------------------------------------------------------------------------

// TODO(NockappCatalogStore): backend that stores the catalog inside a
// long-running NockApp via `nockapp-grpc` (the gRPC bridge in
// `nockchain/crates/nockapp-grpc`). Cataloging would survive facilitator
// restarts via the NockApp's checkpoint, and would interoperate with any
// other Nockchain-aware service holding the same kernel. Outstanding
// design questions: schema-evolution story (kernel upgrades vs catalog
// rows) and how to expose the per-resource `last_updated` timestamp
// without inflating the kernel state. Tracked outside the Phase 5 scope.

// TODO(KademliaCatalogStore): peer-to-peer backend over
// `nockchain-libp2p-io::kad`. Each facilitator instance would advertise
// its locally cataloged resources into the DHT, and `list` would do a
// fan-out lookup. This is a deliberate non-goal for Phase 5 — the right
// abstraction depends on whether the bazaar evolves into a federated
// gossip layer or stays per-facilitator with cross-facilitator sync as a
// separate concern. Mentioned here to anchor the trait's design intent.
