# Architecture

The architectural shape of `x402-nockchain` — how the seven crates compose, where the protocol boundary sits, and why specific decisions were made.

This document is the entry point for "explain how it's organized." For per-crate API details, read the crate-level READMEs under `crates/*/README.md`.

## Goals + scope

`x402-nockchain` is a Rust implementation of the x402 v2 agentic-payments protocol and the Bazaar discovery extension, adapted to Nockchain. It provides everything a Nockchain-aware operator needs to:

- **Advertise** paywalled HTTP / MCP resources via `bazaar` blocks attached to `402 Payment Required` responses.
- **Pay** for those resources by constructing signed `PaymentPayload`s.
- **Verify + settle** the resulting payments — running a facilitator that checks the §5.4.1 envelope, submits signed transactions to the chain, and serves Bazaar `/discovery/resources` queries.
- **Discover** previously-cataloged services via the same facilitator's discovery endpoint.

It deliberately is *not*:

- A Nockchain wallet (we consume the existing `nockchain-wallet` daemon / kernel rather than re-implementing it).
- A multi-chain x402 facilitator (the protocol-neutral crates can power one, but this workspace's `x402-nockchain-*` crates target Nockchain only).
- An on-chain extension (the chain's `sig_hash` and `tx-id` Hoon kernels are sufficient; we call into them, we don't add new modes — see ADR-0010).

## The protocol boundary

The workspace is split along a **protocol-first** boundary. Crates with the bare `x402-*` prefix are *protocol-neutral* — they hold types, traits, and orchestration that any x402 implementation could reuse. Crates prefixed `x402-nockchain-*` are *Nockchain-bound* — they couple to Nockchain's curve (Cheetah), hash (Tip5), wallet daemon, and chain RPC.

```
            ┌─────────────────────────────────────────────────────┐
protocol-   │  x402-types     x402-advertiser   x402-client       │
neutral     │      ▲                ▲               ▲             │
            │      │  Signer trait  │  WalletBackend trait        │
            └──────┼────────────────┼───────────────┼─────────────┘
                   │                │               │
            ┌──────┼────────────────┼───────────────┼─────────────┐
Nockchain-  │  x402-nockchain-crypto   x402-nockchain-wallet-client│
bound       │                                  │                  │
            │  x402-nockchain-facilitator (consumes both)          │
            └──────────────────────────────────────────────────────┘
```

The protocol-neutral crates know nothing about Cheetah, Tip5, or `RawTx`. They define the *shape* (`PaymentPayload`, `Authorization`, `WalletBackend`) and the Nockchain-bound crates *implement* it. This lets a future EVM or Solana adapter substitute its own crypto + wallet crate without touching the trait definitions or the facilitator's HTTP layer. (ADR-0001, ADR-0002)

## Crate topology

### Protocol-neutral crates

#### `x402-types`
Serde types for every wire-shape in x402 v2 + Bazaar v2:
- `PaymentRequirements` — the `accepts[]` entry on a 402 response.
- `PaymentPayload<P>` — generic over the scheme-specific payload `P`. The default `P = serde_json::Value` keeps the facilitator decoupled from any specific network's payload shape; the `nockchain` feature exports `ExactNockchainPayload` for the `(exact, nockchain:*)` scheme.
- `Authorization` — the signed inner struct that commits the facilitator non-custodially (`to`, `value`, `fee`, `notes`, `change_address`).
- `NoteRef` + `NoteLock` — input-UTXO references; `NoteLock` discriminates `SimplePkh` vs `CoinbasePkh { timelock_min }` (Phase 6 / ADR-0017).
- `SchnorrSignatureJson` — base58 pubkey + 8-Belt `chal`/`sig` arrays.
- `BazaarExtension`, `DiscoveryResource`, `DiscoveryInfo` — the bazaar shapes.

The crate roundtrips every JSON example in `docs/specs-snapshot/` byte-identically (after key-order canonicalization).

#### `x402-advertiser`
Server-side builders that attach `bazaar` extension blocks to 402 responses. `declare_http_query`, `declare_http_body`, `declare_mcp` — each emits a typed `BazaarExtension` plus the `extra` block the facilitator expects to see when cataloging.

#### `x402-client`
Client-side helpers + extensibility seams:
- 402-response parsing.
- `build_exact_nockchain_payload` — the *path-2A* envelope-only payload builder.
- `build_exact_nockchain_payload_with_wallet` — the *path-2B* builder; delegates `RawTx` assembly to a `WalletBackend`.
- `BazaarClient` — typed queries against `/discovery/resources`.
- `Signer` trait — the §5.4.1 envelope-signing seam.
- `WalletBackend` trait — the path-2B chain-tx-assembly seam (ADR-0010).
- `StubSigner`, `StubWalletBackend` — zero-sig implementations for tests.

#### `x402-mcp`
A small registry that wraps tool implementations + their `PaymentRequirements`. On unpaid calls it returns a 402 with a correctly-assembled `bazaar` block; on paid calls it proxies to the tool. Mounts as an `axum::Router` so MCP servers built on top can plug it in directly.

### Nockchain-bound crates

#### `x402-nockchain-crypto`
The cryptographic substrate. Implements:
- `SchnorrPrivateKey` — wrapped scalar, with `sign` / `verify` / `public_key`.
- `NockchainSigner` — `x402-client::Signer` impl that signs `Authorization`s under the `x402-nockchain-v2` Tip5 domain separator (ADR-0009).
- `NockchainVerifier` — used by the facilitator's `/verify`.
- `siwn::*` — Sign-In-With-Nockchain (CAIP-122) under the `siwn-v1` domain.
- `ReplayCache` + `InMemoryReplayCache` — pluggable replay protection (ADR-0015).

The crate carries an in-tree port of Nockchain's math primitives (Belt, F6, Cheetah, Tip5) under `src/math/`, behind a feature flag — see ADR-0007. Cross-backend golden-vector tests pin byte-equivalence with chain-side signatures.

#### `x402-nockchain-wallet-client`
The reference `WalletBackend` implementation. Embeds a Nockchain wallet kernel (a `NockApp` booted from kernel JAM bytes the caller supplies via `X402_KERNEL_JAM`) and drives `%sig-hash` + `%tx-id` pokes to assemble a chain-verifiable signed `RawTx`.

The eleven-step `authorize_and_sign` flow mirrors `hull-llm/src/tx_builder.rs::build_settlement_tx` for steps 4–9 (the canonical reference), then adds the §5.4.1 envelope sig in step 11. The crate ships **no kernel bytes** — distribution is the caller's responsibility, keeping the compile-time footprint small (`docs/wallet-integration.md §4`).

#### `x402-nockchain-facilitator`
The HTTP service. Built on `axum`. Endpoints:
- `POST /verify` — runs §6.4 verification; catalogs `bazaar` blocks if present.
- `POST /settle` — re-verifies, then submits via the gRPC `ChainClient` (path-2B) or returns `chain_unimplemented` (path-2A envelope-only).
- `GET /discovery/resources` — paginated query against the `CatalogStore` with `type`, `network`, `scheme` filters.
- `GET /metrics` — Prometheus scrape endpoint.
- SIWN middleware on `/discovery/resources` (ADR-0015).
- `tracing` spans across the request lifecycle (ADR-0016).

## Request flows

### 402 → pay → retry → discover

```
   client                resource-server         facilitator
     │                          │                     │
     │── GET /widget ─────────► │                     │
     │ ◄── 402 + bazaar ──────  │                     │
     │                          │                     │
     │── /discovery/resources ─────────────────────►  │  (optional)
     │ ◄── DiscoveryResource[] ────────────────────── │
     │                          │                     │
     │  (sign Authorization with NockchainSigner)     │
     │                          │                     │
     │── GET /widget                                  │
     │   PAYMENT-SIGNATURE: <PaymentPayload> ──────►  │
     │                          │                     │
     │                          │── /verify ────────► │
     │                          │ ◄── 200 OK ──────── │
     │                          │── /settle ────────► │
     │                          │ ◄── ChainStatus ─── │  (Accepted | Processing | Rejected)
     │                          │                     │
     │ ◄── 200 + widget ──────  │                     │
```

### `/verify` pipeline

The facilitator runs the §6.4 checks in order, each early-exiting on failure with a typed error:

1. **Schema** — `x402Version == 2`, `scheme` + `network` match.
2. **Amount** — `value ≥ maxAmountRequired`, `fee ≥ minFee`.
3. **Recipient** — `to == payTo`.
4. **Time** — `validAfter`/`validBefore` within ±30s clock tolerance.
5. **Nonce** — not previously settled (replay cache).
6. **Notes** — every `auth.notes[i]` exists, is unspent, and the lock is satisfiable by `signature.pubkey`.
7. **Signature** — Schnorr verify against the §5.4.1 sign-message digest.
8. **PKH binding** — `Tip5(pubkey-bytes) == authorization.from` (envelope-form, ADR-0018).

If a `bazaar` extension is present in the payload's `extensions`, the resource is upserted into the `CatalogStore` with the `metadata` echoed back per ADR-0013.

### `/settle` pipeline (path-2A vs path-2B)

```
   /settle handler
     │
     │── re-verify (same checks as /verify) ──┐
     │                                        ├── reject early on any failure
     │                                        │
     │── extract signedRawTx from payload? ───┴───┐
     │                                            │
     │  No  →  ChainStatus::Unimplemented (path-2A envelope-only)
     │
     │  Yes →  decode surrogate, submit_and_wait via gRPC ChainClient
     │                                                   │
     │                                                   ├── Accepted
     │                                                   ├── Processing (poll timeout)
     │                                                   └── Rejected (chain-side error)
```

Path-2A keeps the original ADR-0009 framing — the facilitator is purely a verifier; settlement is a separate operation on the client. Path-2B (ADR-0010) folds the on-chain submission into the facilitator's `/settle` by accepting a fully-signed `RawTx` from the client. Both modes coexist; the handler picks based on whether the payload's `signed_raw_tx` field is populated.

## Two signatures, two purposes

Every payment carries two distinct Schnorr signatures over related-but-non-equal digests:

| Signature | Digest | Domain | Verifier | Purpose |
|---|---|---|---|---|
| **§5.4.1 envelope** | Tip5 sponge over the canonical `Authorization` | `"x402-nockchain-v2"` | `NockchainVerifier` (off-chain) | Proves the payer authorized this exact x402 envelope. |
| **Chain `sig_hash`** | Tip5 sponge over `(seeds, fee)` | n/a (Hoon kernel `%sig-hash` poke) | Chain consensus on `Spend1.witness.pkh_signature` | Proves the spender authorized this exact tx-shape. |

Conflating them — using the §5.4.1 signature where the chain expects the `sig_hash` signature, or vice versa — produces valid-looking payloads that fail at the wrong layer. ADR-0009 names the distinction; path-2B's job is to ship both.

The same `[Belt; 8]` secret signs both, so there's no key-management complexity — only two Tip5 sponges over different inputs.

## Two PKHs, one key

Closely related: `Tip5(pubkey)` admits two non-equivalent canonical forms (Phase 5B Finding #2 / ADR-0018):

| Form | Computation | Used for |
|---|---|---|
| **Envelope-form** (bytes) | `Tip5(raw 97-byte CheetahPoint encoding)` | `authorization.from` (§5.4.1 envelope binding) |
| **Chain-form** (noun) | `Tip5(noun-encoded pubkey via hash-hashable:tip5)` | `Spend1.witness.pkh_signature.hash` + coinbase `lock_root` |

Both hash the same key but produce different base58 strings. `NockchainWalletClient` derives both up-front and exposes them via `payer_pkh()` (envelope-form, what `auth.from` binds) and `payer_pkh_chain_b58()` (chain-form, what coinbase locks use). Wallet integrators MUST carry both.

## Path-2A vs Path-2B

ADR-0010 decides between two designs for bridging the §5.4.1 envelope to a chain-verifiable `Spend1.witness.pkh_signature`:

- **Path 1** (rejected as default) — Hoon-side extension: chain learns a `%x402-sig-hash` mode. Concentrates tx-shape authority in the facilitator and requires every wallet to learn a new signing mode.
- **Path 2A** (envelope-only) — Client signs only the §5.4.1 envelope; settlement is an out-of-band operation. The original Phase-3/4 design.
- **Path 2B** (chosen) — Client uses a `WalletBackend` to assemble + sign the full `RawTx` *and* the §5.4.1 envelope, then bundles the signed `RawTx` in the payload. The facilitator becomes a thin relay — it verifies the envelope and submits the bytes the client already signed.

Path-2B keeps the facilitator non-custodial *and* lets `/settle` close the on-chain loop. The two sub-modes (2A and 2B) coexist in the wire shape — the `signed_raw_tx` field is optional — so a path-2A client and a path-2B facilitator compose, and vice versa.

## Lock-type dispatch (`NoteLock`)

The chain hashes `coinbase_pkh(pkh, timelock_min)` and `simple_pkh(pkh)` to distinct `lock_root` values. A wallet that builds the wrong `SpendCondition` for an input note produces a witness the chain silently drops at the mempool layer (Phase 5B Finding #3, ADR-0017).

`NoteRef.lock` carries this metadata explicitly:

```rust
pub enum NoteLock {
    SimplePkh,                              // default; pre-Phase-6 wire shape
    CoinbasePkh { timelock_min: u64 },      // coinbase-locked input
}
```

`NockchainWalletClient::authorize_and_sign` step 4 dispatches on `note.lock` via `build_input_condition`, mirroring `hull-llm::tx_builder::build_settlement_tx`'s `is_coinbase` branch. The field is `#[serde(default, skip_serializing_if = "is_simple_pkh")]` so prior-Phase-6 payloads round-trip byte-identically.

## Wallet integration model

Path-2B wallets integrate via the `WalletBackend` trait in `x402-client`:

```rust
#[async_trait]
pub trait WalletBackend: Send + Sync {
    fn payer_pkh(&self) -> String;            // envelope-form PKH
    fn payer_pubkey_base58(&self) -> String;
    async fn authorize_and_sign(&self, auth: &Authorization)
        -> Result<AuthorizedPayment, WalletBackendError>;
}
```

The reference impl is `NockchainWalletClient` in `x402-nockchain-wallet-client`. It owns three collaborators:

1. A `WalletKernelHandle` wrapping a caller-booted `NockApp` running the wallet kernel (kernel bytes loaded from `X402_KERNEL_JAM`).
2. A `nockchain_client_rs::ChainClient` for off-band balance queries.
3. The Schnorr secret as `[Belt; 8]`.

`authorize_and_sign` runs the eleven-step pipeline and returns `AuthorizedPayment { envelope_signature, signed_raw_tx }`. The caller bundles both into the `PaymentPayload`.

`docs/wallet-integration.md` is the living tracker for wallet-impl status (currently: `NockchainWalletClient` shipping; `x402-client-iris` queued behind `iris-crypto` API stabilization).

## Storage abstraction

The catalog is fronted by the `CatalogStore` trait in `x402-nockchain-facilitator`:

```rust
#[async_trait]
pub trait CatalogStore: Send + Sync {
    async fn upsert(&self, resource: &DiscoveryResource) -> Result<()>;
    async fn list(&self, params: &ListDiscoveryResourcesParams)
        -> Result<(Vec<DiscoveryResource>, u32)>;
    // ...
}
```

Two impls ship in-tree:

- **`SqliteCatalogStore`** — the default for real deployments. Schema in `crates/x402-nockchain-facilitator/migrations/0001_catalog.sql`; migrations applied via `sqlx::migrate!` at startup.
- **`InMemoryCatalogStore`** — zero-dep, used by the contract test suite and demos.

A shared contract test suite at `tests/catalog_store_contract.rs` runs the same assertion set against both impls so adding a new backend is a "make this contract test pass" exercise. The next backend sketched as a TODO is `NockappCatalogStore` over `nockapp-grpc`. (ADR-0005)

## Observability

Per ADR-0016:

- **`tracing` spans** at `request_received`, `signature_verified`, `catalog_upsert`, `settlement_submit`, `confirmation_polled`, `response_emitted`. JSON formatter via `tracing-subscriber`; layered so operator-side stacks (OpenTelemetry, Loki, etc.) can plug in.
- **Prometheus metrics** via `metrics-exporter-prometheus`. Counters by endpoint + result bucket; histograms for verify / settle / list latency. Cardinality-bounded — no PKH or resource-URL labels.
- **`/metrics` endpoint** on the facilitator's axum router (separate from x402 routes).

## Bazaar extension model

The Bazaar extension is "the facilitator IS the registry" — there is no separate registry service. Cataloging is a side effect of `/verify` or `/settle`, so any 402 with a `bazaar` block in `extensions` automatically propagates into the `CatalogStore` after a successful verify.

Three crate-pieces collaborate:

- **`x402-advertiser`** — server-side builders. `declare_mcp(tool, schema, requirements)` returns a `PaymentRequirements` with the right `extensions.bazaar` block + the `extra` metadata the facilitator catalogs.
- **`x402-mcp`** — auto-generates these for an MCP tool registry.
- **`x402-nockchain-facilitator`** — extracts `bazaar.info` from the verified payload, validates against `bazaar.schema` (JSON Schema Draft 2020-12), and upserts into the `CatalogStore`.

Two Nockchain-local extensions are documented as proposed-upstream in ADR-0013:

- A `metadata` field on `DiscoveryResource` echoing the original `DiscoveryInfo` back so consumers don't need to reverse-engineer the schema.
- `network=<…>` and `scheme=<exact|upto>` query parameters on `/discovery/resources`.

Both ship in-tree; upstream filing is gated by the engagement posture.

## Spec-snapshot pinning

ADR-0012 is the governing constraint on what counts as the wire contract. The frozen copies under `docs/specs-snapshot/` are authoritative — upstream PR #102's evolution does **not** auto-propagate. Any divergence between `docs/specs-snapshot/05-payload.md` and the live PR #102 is captured as a snapshot diff and resolved deliberately.

This decouples in-tree evolution from upstream PR cadence (PR #102 has been dormant since 2026-02-18) and gives ADRs like 0013, 0017, 0018 a stable basis for proposing extensions: each extension says "against snapshot version X, propose Y."

## Test strategy

| Layer | Where | Run by |
|---|---|---|
| Unit tests | Per-crate `#[cfg(test)] mod tests` blocks | `cargo test --workspace` (default) |
| Integration tests (in-process) | `crates/x402-nockchain-facilitator/tests/e2e_*.rs` | `cargo test --workspace` |
| Contract test suite (storage) | `crates/x402-nockchain-facilitator/tests/catalog_store_contract.rs` | `cargo test --workspace` (runs against both `Sqlite` + `InMemory`) |
| Cross-backend golden vectors | `crates/x402-nockchain-crypto/tests/golden.rs` | `cargo test --workspace` (pinned signatures + cross-backend byte-equivalence) |
| Path-2B fakenet matrix | `crates/x402-nockchain-wallet-client/tests/fakenet_path2b.rs` | Operator-only; gated `_wallet_kernel_tests` + `#[ignore]`. See ADR-0017. |

The matrix retires the residual risk that the Hoon-side decoder might silently reject canonical multi-seed jams. A successful matrix rerun is the operational proof that path-2B is end-to-end-good against a real chain.

## ADR index

ADRs are the durable record of decisions that constrain implementation. Read these before re-litigating a design choice.

| # | Subject |
|---|---|
| 0001 | Seven-crate workspace; protocol-neutral / Nockchain-bound split. |
| 0002 | Protocol-first naming (`x402-*` / `x402-nockchain-*`). |
| 0003 | Public standalone repo; consumers use git deps. |
| 0004 | Pin `nockchain-math` by SHA, wrapped behind `x402-nockchain-crypto`. |
| 0005 | SQLite first; `CatalogStore` trait extracted at Phase 5. |
| 0006 | Rust x402 ecosystem landscape + interop posture. |
| 0007 | In-tree port of Cheetah/Tip5/Schnorr math; cross-backend parity test. |
| 0008 | Toolchain pin rationale. |
| 0009 | Two distinct signatures over related-but-different digests. |
| 0010 | Path-2B chosen — client-assembled signed `RawTx` via `WalletBackend`. |
| 0011 | x402 v2 only; no v1 shims. |
| 0012 | `docs/specs-snapshot/` is the contract; upstream evolution doesn't auto-propagate. |
| 0013 | `metadata` echo + `network` / `scheme` filter params. Proposed-upstream. |
| 0014 | Hand-rolled JSON Schema; `schemars` evaluation deferred. |
| 0015 | SIWN replay cache: in-memory TTL default; pluggable trait. |
| 0016 | Structured `tracing` + Prometheus `/metrics`. |
| 0017 | Six-row fakenet path-2B matrix; closed `Accepted` 2026-04-25. Findings → `NoteLock` lift. |
| 0018 | `Tip5(pubkey)` envelope-form vs chain-form canonicalization. Proposed §6.4.7 spec text. |

## Where to next

- **Operating a facilitator** — `crates/x402-nockchain-facilitator/README.md`.
- **Building a wallet integration** — `docs/wallet-integration.md` (contract + status matrix) + `crates/x402-nockchain-wallet-client/src/lib.rs` (reference impl).
- **Cataloging a service** — `crates/x402-advertiser/README.md` + `crates/x402-mcp/README.md`.
- **Running the matrix** — `vesl-agent/harness/fakenet/runbook.md` (proprietary).
