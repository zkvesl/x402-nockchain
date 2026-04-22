# ADR-0003: Standalone workspace; consumers integrate via git dependency

## Status

Accepted — 2026-04-22.

## Context

This codebase is a Rust implementation of x402 + Bazaar for Nockchain. Consumers (applications, facilitator operators, tooling) need a stable integration surface. The question: how is the workspace hosted and consumed?

Options considered:

1. Embed the crates inside a downstream application's repo. Simplest for a single-consumer case, but locks out external adopters and couples release cycles to one application's roadmap.
2. Vendor the crates into `nockchain/nockchain` as a subdirectory. Requires upstream coordination and would entangle release cadence with the chain repo itself.
3. **Standalone repo** with consumers pulling via git dependency.
4. Publish every crate to crates.io immediately. Premature for pre-1.0 greenfield work — leaves no room to iterate on public API without version churn.

## Decision

Host the six crates as a single standalone Cargo workspace in this repo. Consumers integrate via git dependency:

```toml
[dependencies]
x402-client = { git = "https://github.com/zkvesl/x402-nockchain" }
x402-types  = { git = "https://github.com/zkvesl/x402-nockchain" }
```

Consumers pin to specific commits or tags. Crates.io publishing for the four network-neutral crates (`x402-types`, `x402-advertiser`, `x402-client`, `x402-mcp`) remains a deferred option and will be revisited in a future ADR when the public API has stabilized.

## Consequences

**Benefits.**
- Infrastructure code is decoupled from any consumer project's roadmap.
- External adopters can depend on this workspace directly without organizational coordination.
- Release cadence is independent of both chain upgrades and any single operator's product cycles.
- Preserves the option to split individual crates into their own repos (or publish to crates.io) without reshaping the development workflow.

**Costs.**
- Consumer projects carry a git-dep coordination burden (lock files, SHA pinning) until crates.io publishing is authorized.
- CI for consumers must be able to reach this repo's remote during builds.
- Any commit to this repo is public (or will be). Secrets, internal URLs, and operator-specific framing do not belong here.
