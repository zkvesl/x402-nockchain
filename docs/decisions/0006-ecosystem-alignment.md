# ADR-0006: Ecosystem alignment — relationship to other Rust x402 implementations

## Status

Accepted — 2026-04-22.

## Context

A sweep of the public Rust x402 ecosystem (GitHub code and repository search, 2026-04-22) surfaced the following related work:

| Project | Scope | Networks | Facilitator? | Distribution |
|---------|-------|----------|--------------|--------------|
| [`bitrouter/x402-kit`](https://github.com/bitrouter/x402-kit) (AIMOverse) | Composable SDK: core types, signer, paywall middleware, network adapters, an extensions crate (with a Bazaar module), framework-agnostic | EVM (and potentially others via `x402-networks`) | No — README explicitly states "x402-kit is not a facilitator — it's a composable SDK for buyers (signers) and sellers (servers)." | crates.io: `x402-kit`, `x402-core`, `x402-signer`, `x402-paywall` |
| [`stripe/purl`](https://github.com/stripe/purl) | X402 protocol V2 types module inside a larger codebase | EVM | No | Internal to `purl` |
| [`solana-foundation/templates/community/x402-solana-rust`](https://github.com/solana-foundation/templates) | Solana-specific Rust template | SVM | No | Template only |
| [`solana-foundation/moneymq`](https://github.com/solana-foundation/moneymq) | Event system with x402 agentic-payment events | SVM | No | Application |
| Smaller projects (`qntx/r402`, `ethereumdegen/defi-relay-code`, `zh/cashr`, `zh/solw`, `Calhooon/dolphinmilk`, `Abraxas1010/agenthalo`, `broomva/life`, etc.) | Various chain-specific clients and wrappers | EVM, SVM, BSV | No | Per-project |

None of these target Nockchain.

The Coinbase upstream repository (`coinbase/x402`) itself has reference implementations in Go, Python, and TypeScript only — no Rust.

## Decision

This workspace proceeds independently of these projects, with three specific stances:

1. **Types are anchored to the upstream specification snapshots** (`docs/specs-snapshot/`), not to any third-party Rust type definition. This keeps the workspace spec-compliant by construction, insulated from third-party API drift, and avoids taking a dependency on a crate we do not control.
2. **The `Signer` trait in `x402-client` is the extensibility seam**. It lets external signer implementations (including compatible third-party SDKs) plug in without us forking or depending on them. A consumer who already uses `x402-kit` for EVM/SVM signing can write a thin adapter to present their signer as an `x402_client::Signer` and use this workspace's facilitator end-to-end.
3. **Crate namespace is disjoint.** Our published crate names (`x402-types`, `x402-advertiser`, `x402-client`, `x402-mcp`, `x402-nockchain-crypto`, `x402-nockchain-facilitator`) do not collide with any crates already published on crates.io by other Rust x402 projects.

## Interop paths (available, not committed)

These remain explicit future options, to be revisited after the workspace's own public API stabilizes:

- **Consume `x402-core` instead of maintaining `x402-types`.** Viable if and only if `x402-core`'s types align byte-for-byte with the spec-snapshot. A future ADR would record the survey result and the switchover if taken.
- **Publish a `x402-nockchain-signer-for-x402-kit` adapter crate.** Would let `x402-kit`'s reqwest-middleware buyers pay for Nockchain-served resources with a one-line dependency addition on their side.
- **Cross-reference the Bazaar extension shape.** Both this workspace and `x402-kit::x402-extensions` implement the Bazaar discovery extension. If the wire formats diverge non-trivially, we converge (we are spec-snapshot-anchored and will propose upstream where applicable).

## Consequences

**Benefits.**
- Independence now preserves the option to interop later without lock-in.
- Spec-snapshot anchoring is the right stance for a first-mover implementation — correctness against the chain specification matters more than ecosystem coupling.
- The facilitator niche is uncontested in Rust today. This workspace can take that slot cleanly.

**Costs.**
- Duplication of effort: some data types overlap with `x402-core::transport`. Acceptable because the spec-snapshot source of truth is different from their source of truth.
- Adopters who already use `x402-kit` for EVM/SVM need a small adapter to consume our facilitator. The `Signer` trait makes this cheap but non-zero.
- The Rust x402 ecosystem will need deliberate coordination as it grows. Future ADRs will record interop decisions individually rather than try to anticipate them all now.
