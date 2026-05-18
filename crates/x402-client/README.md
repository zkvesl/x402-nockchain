# x402-client

Client-side helpers for the x402 protocol: 402 parsing, `PaymentPayload` construction, the `BazaarClient` for `/discovery/resources`, and the `Signer` / `WalletBackend` traits that let network adapters plug in.

Network-neutral. The Nockchain-specific signer lives in [`x402-nockchain-crypto`](../x402-nockchain-crypto); a reference path-2B wallet backend lives in [`x402-nockchain-wallet-client`](../x402-nockchain-wallet-client).

## Surface

| Type | Role |
|---|---|
| `Signer` | Trait — sign an `Authorization`. Plug-in seam for network-specific signers. |
| `StubSigner` | Returns all-zero signatures. Wire-shape smoke tests only — rejected by the Phase-3 facilitator. |
| `WalletBackend` | Trait — path-2B wallet contract per [ADR-0010](../../docs/decisions/0010-path-2b-client-assembled-rawtx.md). Returns both the §5.4.1 envelope signature and a fully-signed chain `RawTx`. |
| `StubWalletBackend` | All-zero `WalletBackend` for tests. |
| `BazaarClient` | Typed client for `GET /discovery/resources` (with `network` / `scheme` filter support per ADR-0013). |
| `X402Client` | High-level orchestrator: 402 → verify → retry loop. |
| `build_exact_nockchain_payload` | Envelope-only payload builder (path-2A). |
| `build_exact_nockchain_payload_with_wallet` | Path-2B payload builder; delegates to a `WalletBackend`. |

## Bring-your-own-`Signer`

The `Signer` trait is async + object-safe; any signer (Schnorr, secp256k1, hardware wallet) plugs in by implementing two methods. The trait takes both an `Authorization` (which the implementer signs) and the matching `PaymentRequirements` so wrapper signers can apply caller-side policy without re-deriving them — see `x402_client::PolicyEnforcedSigner`. The §5.4.1 digest is over `Authorization` alone (per `06-facilitator.md §6.4`); concrete signers ignore `requirements`.

```rust
use async_trait::async_trait;
use anyhow::Result;
use x402_client::Signer;
use x402_types::payment::{Authorization, PaymentRequirements, SchnorrSignatureJson};

struct MySigner { /* keys, etc. */ }

#[async_trait]
impl Signer for MySigner {
    async fn sign_authorization(
        &self,
        _auth: &Authorization,
        _requirements: &PaymentRequirements,
    ) -> Result<SchnorrSignatureJson> {
        // ... compute signature ...
        Ok(SchnorrSignatureJson::all_zero("my-pubkey"))
    }
    fn from_identifier(&self) -> String { "my-pubkey".into() }
}
```

`x402_nockchain_crypto::NockchainSigner` is the reference Nockchain implementation — it computes the §5.4.1 Tip5 sponge and returns a real Schnorr-over-Cheetah signature.

## Querying a facilitator

```rust no_run
# async fn run() {
use x402_client::BazaarClient;
use x402_types::ListDiscoveryResourcesParams;

let client = BazaarClient::new("http://localhost:9000");
let resources = client
    .list_resources(ListDiscoveryResourcesParams {
        kind: Some("mcp".into()),
        network: Some("nockchain:mainnet".into()),
        ..Default::default()
    })
    .await
    .expect("list");
println!("{} cataloged MCP tool(s)", resources.pagination.total);
# }
```

`network` and `scheme` are Nockchain-local proposed-upstream extensions ([ADR-0013](../../docs/decisions/0013-metadata-and-filter-extensions.md)). Clients that don't set them get the upstream-canonical behaviour.
