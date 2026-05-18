# x402-types

Serde types for the [x402](https://github.com/coinbase/x402) agentic-payments protocol and its [Bazaar](https://github.com/coinbase/x402/blob/main/specs/extensions/bazaar.md) discovery extension. Network-neutral core; Nockchain-specific payload variants are gated behind the `nockchain` feature flag.

## Modules

| Module | Contents |
|---|---|
| [`bazaar`](src/bazaar.rs) | `BazaarExtension`, `DiscoveryInfo`, `DiscoveryResource`, `ListDiscoveryResourcesParams` |
| [`payment`](src/payment.rs) | `PaymentRequirements`, `PaymentPayload`, `Authorization`, `SchnorrSignatureJson`, `ExtensionResponsesHeader` |
| [`facilitator`](src/facilitator.rs) | `VerifyRequest` / `VerifyResponse`, `SettleRequest` / `SettleResponse`, `FacilitatorError` |
| [`siwn`](src/siwn.rs) | CAIP-122 / Sign-In-With-Nockchain |
| [`nockchain`](src/nockchain.rs) | `ExactNockchainPayload`, `SignedRawTx` (Nockchain feature only) |

## Feature flags

| Flag | Default | Effect |
|---|---|---|
| `nockchain` | on | Compiles the Nockchain-specific payload module (`ExactNockchainPayload`, `SignedRawTx` JSON surrogate) |

Disable for pure protocol consumers (clients that only handle the network-neutral envelope) by setting `default-features = false`.

## Usage

```rust
use x402_types::{PaymentRequirements, PaymentPayload};
use serde_json::json;

let requirements = PaymentRequirements {
    scheme: "exact".into(),
    network: "nockchain:mainnet".into(),
    max_amount_required: "65536".into(),
    resource: "https://api.example.com/echo".into(),
    asset: "NOCK".into(),
    pay_to: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".into(),
    max_timeout_seconds: 60,
    description: Some("MCP echo".into()),
    mime_type: Some("application/json".into()),
    output_schema: None,
    extra: Some(json!({ "minFee": "10" })),
    extensions: None,
};
assert_eq!(requirements.scheme, "exact");
```

## Stability

Types track the frozen spec snapshots under [`docs/specs-snapshot/`](../../docs/specs-snapshot/). Per [ADR-0012](../../docs/decisions/0012-spec-snapshot-pinning.md), snapshot refreshes are explicit, reviewed events; upstream PR-#102 evolution does not auto-propagate. Crate version is `0.0.x` until upstream stabilises.

The Nockchain-local extensions (`metadata` field on `DiscoveryResource`; `network` and `scheme` query params on `ListDiscoveryResourcesParams`) are documented as proposed-upstream in [ADR-0013](../../docs/decisions/0013-metadata-and-filter-extensions.md).
