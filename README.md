# x402-nockchain

Reference Rust implementation of the [x402](https://github.com/coinbase/x402) agentic-payments protocol and its [Bazaar](https://github.com/coinbase/x402/blob/main/specs/extensions/bazaar.md) service-discovery extension, adapted to Nockchain.

**Status:** early development. No stable surface yet.

## Layout

A Cargo workspace of six crates, split along a **protocol-first** boundary — network-neutral pieces carry the bare `x402-*` prefix, Nockchain-bound pieces carry `x402-nockchain-*`:

| Crate | Role | Network-bound? |
|---|---|---|
| [`x402-types`](crates/x402-types) | Serde types: `PaymentRequirements`, `PaymentPayload`, `BazaarExtension`, `DiscoveryResource`, etc. | No (Nockchain variants behind a `nockchain` feature) |
| [`x402-advertiser`](crates/x402-advertiser) | Server-side builders (`declare_http_query`, `declare_http_body`, `declare_mcp`) and middleware to attach `bazaar` blocks to 402 responses. | No |
| [`x402-client`](crates/x402-client) | Client-side: 402 parsing, `PaymentPayload` construction, `BazaarClient` for `/discovery/resources`. Defines the `Signer` trait that is the extensibility seam for network adapters. | No |
| [`x402-mcp`](crates/x402-mcp) | MCP adapter: wraps a tool registry and emits 402 responses with bazaar blocks for unpaid tool calls. | No |
| [`x402-nockchain-crypto`](crates/x402-nockchain-crypto) | Schnorr-over-Cheetah signing/verification under Tip5 domain separators (`x402-nockchain-v2`, `siwn-v1`). Implements `x402-client::Signer`. Thin wrapper over Nockchain's `nockchain-math`. | **Yes** |
| [`x402-nockchain-facilitator`](crates/x402-nockchain-facilitator) | Axum service implementing `/verify`, `/settle`, `/discovery/resources`. SQLite catalog. SIWN auth middleware. | **Yes** |

## Upstream alignment

- **x402 v2** per [`nockchain/nockchain#102`](https://github.com/nockchain/nockchain/pull/102) (spec port) and the protocol sections in `coinbase/x402:specs/`.
- **Bazaar** per [`coinbase/x402:specs/extensions/bazaar.md`](https://github.com/coinbase/x402/blob/main/specs/extensions/bazaar.md).
- Architectural decisions recorded under [`docs/decisions/`](docs/decisions/) (ADRs).
- Spec snapshots (frozen copies of upstream source specs used during implementation) under [`docs/specs-snapshot/`](docs/specs-snapshot/).

## Building

```
cargo check --workspace
cargo test --workspace
cargo run --example e2e_demo
```

The `e2e_demo` example spins a stub facilitator, builds a `bazaar` extension for an MCP tool, catalogs it, and queries the upstream-canonical `/discovery/resources` API to prove a round trip.

## Related work

Other Rust x402 projects exist in the ecosystem; none of them target Nockchain, and most are signer/paywall SDKs rather than facilitators. See [`docs/decisions/0006-ecosystem-alignment.md`](docs/decisions/0006-ecosystem-alignment.md) for the landscape survey and this workspace's interop posture.

## License

Dual-licensed under Apache-2.0 OR MIT.
