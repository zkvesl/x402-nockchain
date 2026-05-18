//! Reference Nockchain x402 facilitator.
//!
//! axum service exposing `/verify`, `/settle`, and `/discovery/resources`,
//! backed by a pluggable [`CatalogStore`]. Cataloging happens as a side
//! effect of `/verify` (and `/settle`) per
//! `coinbase/x402:specs/extensions/bazaar.md §Facilitator Behavior` —
//! "the facilitator IS the registry".
//!
//! Phase 3 replaces the Phase-2 all-zero signature stub with a real
//! Schnorr-over-Cheetah verifier
//! ([`x402_nockchain_crypto::NockchainVerifier`]). A `SIGN-IN-WITH-X`
//! SIWN middleware can be attached to `/discovery/resources` via
//! [`router_with_siwn`].
//!
//! Phase 4 introduces a [`ChainClient`] abstraction for `/settle`. The
//! default tests use a [`StubChainClient`] that synthesises a tx_id; a
//! [`GrpcChainClient`] wraps `nockchain_client_rs` for real fakenet runs
//! (see `examples/demo.sh`).
//!
//! Phase 5A extracts the catalog into a [`CatalogStore`] trait
//! ([`SqliteCatalogStore`] + [`InMemoryCatalogStore`]) and adds a
//! `/metrics` Prometheus endpoint via [`observability`].

pub mod chain;
pub mod clock;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod observability;
pub mod scheme;
pub mod storage;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::SqlitePool;
use x402_nockchain_crypto::{InMemoryReplayCache, ReplayCache};

pub use chain::{
    ChainClient, ChainError, ChainSettle, ChainStatus, GrpcChainClient, StubChainClient,
};
pub use clock::{Clock, MockClock, SystemClock};
pub use error::{AppError, VerifyError};
pub use handlers::verify::{verify_envelope, verify_envelope_for_settle};
pub use middleware::siwn::SiwnGate;
pub use observability::{init_tracing, install_metrics_recorder, metrics_router, MetricsHandle};
pub use scheme::{
    ExactNockchainHandler, SchemeHandler, SchemeHandlerRegistry, UptoNockchainHandler,
};
pub use storage::{CatalogStore, InMemoryCatalogStore, SqliteCatalogStore};

/// Default tolerance for clock skew between client and facilitator when
/// applying the `valid_after` / `valid_before` window check. 60s matches
/// upstream x402 conventions; operators with tight session-key rotation
/// may dial this lower via [`AppState::with_clock_skew_tolerance`].
pub const DEFAULT_CLOCK_SKEW_TOLERANCE: Duration = Duration::from_secs(60);

/// Default lifetime of an `Authorization.nonce` in the replay cache. Long
/// enough that a maximally-permissive 5-minute authorization window
/// cannot be replayed within the cache's memory horizon, plus margin.
/// Tuned per `06-facilitator.md §6.6` — the cache MUST outlive any
/// honest authorization's `valid_before`. Operators may dial this up
/// for longer-window deployments via
/// [`AppState::with_replay_ttl`].
pub const DEFAULT_REPLAY_TTL: Duration = Duration::from_secs(15 * 60);

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctest;

/// Shared application state: catalog backend + chain-submission client
/// + verifier-policy plumbing (clock, replay cache, skew tolerance,
/// replay TTL) + scheme-handler registry.
///
/// Clone is cheap; every field is an `Arc`-shared handle or `Copy`.
#[derive(Clone)]
pub struct AppState {
    pub catalog: Arc<dyn CatalogStore>,
    pub chain: Arc<dyn ChainClient>,
    pub clock: Arc<dyn Clock>,
    pub replay_cache: Arc<dyn ReplayCache>,
    pub clock_skew_tolerance: Duration,
    pub replay_ttl: Duration,
    pub scheme_registry: SchemeHandlerRegistry,
}

impl AppState {
    pub fn new(catalog: Arc<dyn CatalogStore>, chain: Arc<dyn ChainClient>) -> Self {
        Self {
            catalog,
            chain,
            clock: Arc::new(SystemClock),
            replay_cache: Arc::new(InMemoryReplayCache::new()),
            clock_skew_tolerance: DEFAULT_CLOCK_SKEW_TOLERANCE,
            replay_ttl: DEFAULT_REPLAY_TTL,
            scheme_registry: SchemeHandlerRegistry::with_default_handlers(),
        }
    }

    /// Convenience: an [`AppState`] whose chain client is a
    /// [`StubChainClient`] and catalog is the supplied store. Used by
    /// tests and the e2e demo.
    pub fn with_stub_chain(catalog: Arc<dyn CatalogStore>) -> Self {
        Self::new(catalog, Arc::new(StubChainClient))
    }

    /// Convenience: SQLite-backed catalog wrapping `pool`, plus a
    /// [`StubChainClient`]. Mirrors the Phase-2/3 test convenience pattern.
    pub fn with_stub_chain_sqlite(pool: SqlitePool) -> Self {
        Self::with_stub_chain(Arc::new(SqliteCatalogStore::new(pool)))
    }

    /// Override the clock — primarily for [`MockClock`]-driven tests.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Override the replay cache — useful when a deployment plugs in a
    /// non-default cache (e.g. a future Sqlite-backed cache from R1.4).
    pub fn with_replay_cache(mut self, cache: Arc<dyn ReplayCache>) -> Self {
        self.replay_cache = cache;
        self
    }

    /// Override the clock-skew tolerance applied to the
    /// `valid_after`/`valid_before` window check. Operators running
    /// session keys with tight rotation may want this dialed below the
    /// 60s default.
    pub fn with_clock_skew_tolerance(mut self, tolerance: Duration) -> Self {
        self.clock_skew_tolerance = tolerance;
        self
    }

    /// Override the replay-cache TTL. Should be at least as long as the
    /// longest `valid_before - valid_after` window the deployment
    /// expects to authorize, plus the clock-skew tolerance.
    pub fn with_replay_ttl(mut self, ttl: Duration) -> Self {
        self.replay_ttl = ttl;
        self
    }

    /// Override the scheme-handler registry. The default registry
    /// ships with `(exact, nockchain:*)` and `(upto, nockchain:*)`
    /// handlers; deployments adding scheme variants build a custom
    /// registry and pass it here.
    pub fn with_scheme_registry(mut self, registry: SchemeHandlerRegistry) -> Self {
        self.scheme_registry = registry;
        self
    }
}

/// Build the axum `Router` exposing the three facilitator endpoints.
///
/// `/verify` and `/settle` are open; `/discovery/resources` is open in
/// this form. See [`router_with_siwn`] for the SIWN-gated variant.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/verify", post(handlers::verify::handler))
        .route("/settle", post(handlers::settle::handler))
        .route("/discovery/resources", get(handlers::discovery::list_resources))
        .with_state(state)
}

/// Like [`router`] but gates `/discovery/resources` behind the SIWN
/// middleware — unauthenticated GETs get `401 Unauthorized`.
pub fn router_with_siwn(state: AppState, gate: SiwnGate) -> Router {
    let gated = Router::new()
        .route("/discovery/resources", get(handlers::discovery::list_resources))
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            gate,
            middleware::siwn::require_siwn,
        ));
    Router::new()
        .route("/verify", post(handlers::verify::handler))
        .route("/settle", post(handlers::settle::handler))
        .with_state(state)
        .merge(gated)
}

/// Connect to a SQLite database at `database_url` and apply every pending
/// migration under `crates/x402-nockchain-facilitator/migrations/`.
///
/// Startup fails — returns `Err` — if the database is unreachable or if
/// any migration cannot apply. Callers MUST propagate the error; a
/// facilitator that silently skipped migrations would desynchronise from
/// the schema its handlers assume.
pub async fn open_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    use anyhow::Context;
    let pool = SqlitePool::connect(database_url)
        .await
        .with_context(|| format!("connect to SQLite at {database_url}"))?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("apply catalog migrations")?;
    Ok(pool)
}

/// Connect to an in-memory SQLite database and run every migration.
/// Convenience helper for tests and the e2e demo.
pub async fn in_memory_pool() -> anyhow::Result<SqlitePool> {
    open_pool("sqlite::memory:").await
}
