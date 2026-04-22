# ADR-0003: Public standalone repo, proprietary consumers

## Status

Accepted — 2026-04-22.

## Context

This codebase is a Rust implementation of x402 + Bazaar for Nockchain. The operator and primary maintainer (vesl) has proprietary adjacent projects (vesl-agent, vesl-cloud) that will consume this code. The question: where does this workspace live?

Options considered:
1. Inside vesl's proprietary vesl-agent repo. Simplest for development flow, but locks out external adopters and misaligns with the intent to become canonical Nockchain infrastructure.
2. Inside `nockchain/nockchain` as a subdirectory. Requires upstream coordination and would entangle release cycles with the chain repo.
3. **Standalone public repo** under a vesl GitHub organization.

## Decision

Host the workspace at **`github.com/zkvesl/x402-nockchain`** as a standalone public repository, independent of all consumer projects.

Consumers integrate via git dependency:

```toml
[dependencies]
x402-client = { git = "https://github.com/zkvesl/x402-nockchain" }
x402-types = { git = "https://github.com/zkvesl/x402-nockchain" }
```

Proprietary consumers (vesl-agent, vesl-cloud) pin to specific commits/tags; crates.io publishing remains a future option subject to a later ADR.

At the time of this decision the repo is private inside the zkvesl org. Flipping visibility to public is a separate explicit decision, recorded in a future ADR when it occurs.

## Consequences

**Benefits.**
- Infrastructure code is decoupled from any single operator's product roadmap.
- External adopters (anyone building on Nockchain x402) can depend on this repo directly.
- Release cadence is independent of both chain upgrades and vesl product cycles.
- Separation of strategy (which stays proprietary in vesl-agent's `docs/plans/`) from rationale (which lives in these ADRs and is safe for public visibility).

**Costs.**
- Consumer projects carry a git-dep coordination burden (lock files, SHA pinning) until crates.io publishing is authorized.
- Sensitive design rationale that touches proprietary product strategy must stay out of these ADRs and live instead in the proprietary plans repo.
- Contributors and maintainers must be mindful that any commit to this repo is, or will be, public — secrets, internal URLs, and strategic framing do not belong here.
