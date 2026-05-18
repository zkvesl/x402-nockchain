# x402-nockchain-facilitator

Reference Nockchain x402 facilitator: an axum service implementing `/verify`, `/settle`, and the upstream-canonical `/discovery/resources` API. SQLite catalog by default (pluggable per [ADR-0005](../../docs/decisions/0005-sqlite-first-storage.md)). SIWN auth middleware. Prometheus `/metrics` endpoint.

## Surface

| Module | Role |
|---|---|
| `handlers::verify` | `POST /verify` — envelope + signature validation per `06-facilitator.md §6.3.1`. |
| `handlers::settle` | `POST /settle` — submits a verified payload through the configured `ChainClient`. |
| `handlers::discovery` | `GET /discovery/resources` — pagination + `type` / `network` / `scheme` filters per [ADR-0013](../../docs/decisions/0013-metadata-and-filter-extensions.md). |
| `handlers::bazaar` | Side-effect upserter — runs after `/verify` and `/settle` to catalog the resource. |
| `chain` | `ChainClient` trait + `StubChainClient` (tests) + `GrpcChainClient` (real Nockchain via `nockchain_client_rs`). |
| `storage` | `CatalogStore` trait + `SqliteCatalogStore` + `InMemoryCatalogStore`. |
| `middleware::siwn` | Optional SIWN auth gate for `/discovery/resources`. |
| `observability` | `init_tracing`, `install_metrics_recorder`, `metrics_router`. |

## Startup

Minimum viable facilitator: in-memory SQLite + stub chain client, no SIWN.

```rust no_run
# async fn run() {
use std::sync::Arc;

use tokio::net::TcpListener;
use x402_nockchain_facilitator::{
    install_metrics_recorder, init_tracing, metrics_router,
    router as facilitator_router, AppState, InMemoryCatalogStore,
};

init_tracing().unwrap();
let metrics = install_metrics_recorder().unwrap();

let catalog = Arc::new(InMemoryCatalogStore::new());
let state = AppState::with_stub_chain(catalog);

let app = facilitator_router(state).merge(metrics_router(metrics));
let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
// `axum::serve(listener, app).await.unwrap();`
let _ = listener;
let _ = app;
# }
```

## Configuration

| Setting | Source | Notes |
|---|---|---|
| Catalog backend | `Arc<dyn CatalogStore>` passed to `AppState::new` | Built-in: `SqliteCatalogStore`, `InMemoryCatalogStore`. Future: NockApp-, DHT-backed. |
| Chain client | `Arc<dyn ChainClient>` passed to `AppState::new` | Built-in: `StubChainClient` (synthesises tx_ids), `GrpcChainClient` (real Nockchain gRPC). |
| Tracing filter | `RUST_LOG` env var | Default `info`. JSON output to stdout. |
| Metrics endpoint | Mounted via `metrics_router(handle)` | Returns Prometheus text-exposition format on `GET /metrics`. |
| SIWN gate | `router_with_siwn(state, gate)` | When used, `/discovery/resources` requires a valid `SIGN-IN-WITH-X` header. |

## Metrics

Phase 5A surface (constants in `observability::metric`):

| Name | Kind | Labels |
|---|---|---|
| `x402_requests_total` | counter | `endpoint`, `outcome` |
| `x402_verify_latency_seconds` | histogram | `outcome` |
| `x402_settle_latency_seconds` | histogram | `outcome` |
| `x402_catalog_list_latency_seconds` | histogram | `outcome` |
| `x402_catalog_upserts_total` | counter | `outcome`, `kind` |

Per [ADR-0016](../../docs/decisions/0016-structured-logging-and-metrics.md), label cardinality is bounded by endpoint × outcome × kind — the surface won't explode under load. PKHs and resource URLs are deliberately not labels.

## Tracing backends

`init_tracing()` installs a JSON-formatted stdout layer with env-filter. Operators who want OTLP / Loki / Tracy register an additional `tracing-subscriber` layer in their entrypoint — the JSON layer is a default, not a constraint.

## See also

- [`crates/x402-types`](../x402-types) — wire types this crate validates.
- [`crates/x402-nockchain-crypto`](../x402-nockchain-crypto) — the Schnorr verifier used in `/verify`.
- [`crates/x402-nockchain-wallet-client`](../x402-nockchain-wallet-client) — path-2B `WalletBackend` reference impl that produces the `SignedRawTx` `/settle` consumes.
- [ADR-0010](../../docs/decisions/0010-path-2b-client-assembled-rawtx.md) — settlement architecture.
- [ADR-0016](../../docs/decisions/0016-structured-logging-and-metrics.md) — observability rationale.
