# ADR-0002: Protocol-first crate naming

## Status

Accepted — 2026-04-22.

## Context

The workspace (per ADR-0001) contains both network-neutral protocol code and Nockchain-specific code. A naming scheme needed to communicate which category each crate falls into — both to future adopters and to any maintainer considering an EVM/SVM adapter in the same workspace.

Candidates considered:
1. `vesl-x402-*` across the board. Binds every crate to the vesl brand.
2. `nockchain-x402-*` across the board. Stakes a bigger ecosystem claim but misrepresents the four neutral crates.
3. **Protocol-first:** bare `x402-*` for neutral crates, `x402-nockchain-*` for chain-bound.
4. `x402` alone (as the repo root) with all crates inside it.

## Decision

Use the **protocol-first** convention:

- Network-neutral crates: `x402-types`, `x402-advertiser`, `x402-client`, `x402-mcp`.
- Nockchain-bound crates: `x402-nockchain-crypto`, `x402-nockchain-facilitator`.
- Hypothetical future EVM/SVM adapters follow the same pattern: `x402-evm-crypto`, `x402-svm-facilitator`, etc.

## Consequences

**Benefits.**
- Naming communicates scope: a consumer reading `Cargo.toml` can immediately see whether a crate is chain-bound.
- Publishing the neutral crates to crates.io (if we decide to) requires no rename.
- Ecosystem adopters on other networks can contribute sibling `x402-<network>-*` crates to this workspace without naming churn.

**Costs.**
- The repo name `x402-nockchain` doesn't capture the four neutral crates' scope, creating a minor mismatch between repo name and contents. We accept this because the repo is Nockchain-focused in operational intent (the facilitator we run) even if the SDK crates are neutral.
- If we later fork a neutral crate off to its own repo (e.g., `github.com/zkvesl/x402-types`), we would need a namespace migration in Cargo.toml consumers — deferred to a future ADR when the question actually arises.
