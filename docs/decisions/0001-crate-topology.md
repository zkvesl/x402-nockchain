# ADR-0001: Six-crate workspace topology

## Status

Accepted — 2026-04-22.

## Context

The x402 protocol surface spans several concerns: serializable types, server-side advertisement helpers, client-side discovery and payload construction, Nockchain-specific cryptographic primitives, a facilitator HTTP service, and framework adapters (MCP first). Two boundary decisions stood out:

1. **One monolithic crate vs. many small ones.** A monolith minimizes workspace complexity but forfeits any future option to publish the protocol surface as a reusable Rust SDK. A shattered layout adds coordination cost without corresponding benefit at pre-1.0.
2. **Where to draw the seam between network-neutral protocol code and Nockchain-specific code.** Mixing them inside a single crate locks the whole codebase to Nockchain; keeping them separate preserves the option for EVM/SVM adapters later without forks.

## Decision

Split the workspace into **six crates** at the following boundaries:

| Crate | Role | Network-bound |
|---|---|---|
| `x402-types` | Serde types (payment, bazaar, siwn, nockchain variants behind a feature flag) | No |
| `x402-advertiser` | Server-side builders and middleware that produce `bazaar` extension blocks | No |
| `x402-client` | Client-side discovery + payload construction; defines the `Signer` trait | No |
| `x402-mcp` | MCP adapter that emits 402 with bazaar blocks for unpaid tool calls | No |
| `x402-nockchain-crypto` | Schnorr-over-Cheetah, Tip5, SIWN wrappers over `nockchain-math`; implements `Signer` | Yes |
| `x402-nockchain-facilitator` | Axum service for `/verify`, `/settle`, `/discovery/resources` with SQLite catalog | Yes |

The `Signer` trait in `x402-client` is the extensibility seam: network adapters provide their own impl (Nockchain here; EVM/SVM hypothetically later) without touching the protocol crates.

## Consequences

**Benefits.**
- Future EVM/SVM adapters can reuse `x402-types`, `x402-advertiser`, `x402-client`, `x402-mcp` unchanged — replace only the two `x402-nockchain-*` crates.
- The four neutral crates become publishable to crates.io as the reference Rust x402 SDK if/when we choose to.
- Types sit in a leaf crate (`x402-types`), so a consumer that only needs JSON (de)serialization can depend on it without pulling the HTTP/TLS stacks transitively.

**Costs.**
- Six Cargo.toml manifests vs. one. Each crate's deps must be curated.
- Cross-crate changes (e.g., adding a shared type) occasionally touch multiple crates.
- CI runs `--workspace` to build/test all six, which is slightly slower than a monolith on cold cache.

**Not considered for future.** Deeper splits (e.g., separating `x402-types-bazaar` from `x402-types-payment`) were rejected — bazaar's `accepts` field composes with `PaymentRequirements`, and splitting there creates a circular-feeling dep dance for zero gain.
