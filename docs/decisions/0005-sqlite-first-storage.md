# ADR-0005: SQLite-first catalog storage; trait extraction deferred to M5

## Status

Accepted — 2026-04-22.

## Context

`x402-nockchain-facilitator` must persist the Bazaar catalog — discovery resources accumulated as a side effect of `/verify` and `/settle` calls. Per `coinbase/x402:specs/extensions/bazaar.md §Facilitator Behavior`, "how a facilitator stores, indexes, and exposes discovered resources is an implementation detail."

Upstream observation (`BAZAAR_UPSTREAM_NOTES.md §6`): each reference impl (Go, Python, TypeScript) rolls its own storage. There is no canonical catalog schema and no replication protocol. Four backends have been floated in passing: database-backed, NockApp-hosted, libp2p Kademlia DHT, and on-chain note data.

Options considered:
1. Build a `CatalogStore` trait up front with multiple implementations to force portability from day one.
2. **Ship SQLite only; extract the trait at M5** once real query patterns are known.
3. Skip storage entirely until M2 or later and use in-memory only. Rejected — M2 is end-to-end demo scope; persistence is core to the facilitator role.
4. Jump directly to NockApp or DHT backends. Rejected — operational complexity before correctness is established.

## Decision

For M2 onwards the facilitator uses **SQLite** (via `sqlx`) as its sole catalog backend. The `CatalogStore` trait is **not** introduced at this point.

At M5 we extract a `CatalogStore` trait from the shape of the real SQLite queries, and add an in-memory implementation for use in tests. Skeleton TODO comments in `x402-nockchain-facilitator` point at `nockapp-grpc` (for a NockApp backend) and `nockchain-libp2p-io::kad` (for a DHT backend) as future work.

## Consequences

**Benefits.**
- Avoids the YAGNI trap of designing a trait around a single hypothetical implementation; the trait that appears at M5 is informed by actual query patterns.
- Single backend through M4 keeps the facilitator tractable while other milestones (real signing, MCP adapter, testnet integration) land.
- SQLite is portable, embeddable, and adequate for catalogs up to ~10^5 entries — comfortably past where the facilitator would need to be anyway before we re-examine pagination design (`BAZAAR_UPSTREAM_NOTES.md §10`).

**Costs.**
- Pre-M5 code is coupled to `sqlx::SqlitePool`. The M5 refactor will need to thread a trait object through the request handlers — non-trivial but mechanical.
- Consumers who want NockApp or DHT-backed storage immediately (e.g., decentralization-focused facilitator operators) cannot pick us up pre-M5. We judge that population to be empty at the time of this decision.
- Schema migrations under `migrations/` will evolve; the M5 trait must accommodate the mature schema, not the M2 one.
