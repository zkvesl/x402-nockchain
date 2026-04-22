# ADR-0004: Nockchain dependency strategy — git-pinned SHA, wrapped behind `x402-nockchain-crypto`

## Status

Accepted — 2026-04-22.

## Context

Nockchain's consensus-critical cryptographic primitives (Cheetah elliptic-curve parameters, Schnorr verification, Tip5 permutation with domain separators) already live in `nockchain/crates/nockchain-math`. The x402 protocol requires payload signatures that are bit-compatible with the chain's verifier — otherwise submitted payments will not validate on-chain.

Options considered for depending on these primitives:
1. Reimplement Cheetah / Schnorr / Tip5 in this workspace. Rejected — reinventing consensus-critical code for no reason, with an ongoing correctness risk every time chain-side parameters evolve.
2. Depend on every relevant nockchain crate directly from every crate that needs them. Rejected — leaks Nockchain types throughout the workspace and couples neutral crates to chain-specific primitives.
3. Publish `nockchain-math` to crates.io as a prerequisite. Rejected — upstream has not signaled intent to publish, and coordinating that would delay our work.
4. **Wrap Nockchain primitives behind a single crate** (`x402-nockchain-crypto`) and have every other crate see only the wrapper's surface.

## Decision

- Take a **git-pinned dependency** on `nockchain/nockchain` (specific SHA, not `master`) from `x402-nockchain-crypto` only.
- `x402-nockchain-crypto` exposes a narrow surface: a `Signer` impl for `x402-client::Signer`, a verifier symmetric to it, SIWN sign/verify, and helpers for Tip5 domain-separator application. No Nockchain types leak out of this crate.
- Every other crate in this workspace that needs signing or verification consumes `x402-nockchain-crypto`'s surface — never `nockchain-math` directly.
- During local development a `[patch.crates-io]` or workspace-level path override may point at a local Nockchain checkout for rapid iteration; CI builds from the pinned SHA.

## Consequences

**Benefits.**
- Bit-compatibility with the chain's verifier is guaranteed by construction.
- Nockchain upgrades are absorbed by rebasing the pin deliberately in one place.
- The four neutral crates remain unaware of Cheetah / Tip5 / Schnorr-over-Cheetah specifics.
- Future EVM/SVM adapters can replace `x402-nockchain-crypto` with their own crate without touching anything else.

**Costs.**
- Git-dep consumers cannot simply `cargo add` us; they must declare the dep with `{ git = "..." }`.
- SHA-pin maintenance is an ongoing operational task — when the chain upgrades, we rebase, re-test the golden signature vectors, and bump the pin.
- CI must be able to reach `github.com/nockchain/nockchain` during builds. If upstream ever restricts access, we mirror.
