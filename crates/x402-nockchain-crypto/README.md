# x402-nockchain-crypto

Nockchain-bound cryptographic primitives for x402. Provides Schnorr-over-Cheetah signing/verification, Tip5 domain separators, the §5.4.1 `sign_message` Tip5 sponge, and SIWN (CAIP-122 / Sign-In-With-Nockchain). Implements [`x402-client::Signer`](../x402-client).

## Surface

| Item | Role |
|---|---|
| `SchnorrPrivateKey` / `SchnorrError` | Wrapped scalar; `from_seed`, `public_key`, `sign`, `verify`. |
| `NockchainSigner` | `x402-client::Signer` impl. Signs `Authorization` per `docs/specs-snapshot/05-payment-payload.md §5.4.1`. |
| `NockchainVerifier` | Verify a `PaymentPayload` against `PaymentRequirements`. Used by the facilitator's `/verify` handler. |
| `x402_sign_message_*` | Tip5 sponge primitives (`x402_sign_message_belts`, `x402_sign_message_digest`, `base58_to_belts`, `pkh_belts_to_base58`). |
| `siwn::*` | `SiwnParams`, `SiwnSigner`, `verify`, `VerifiedIdentity`, `SiwnError`. |
| `ReplayCache` / `InMemoryReplayCache` | Replay-protection trait + default in-memory impl. See [ADR-0015](../../docs/decisions/0015-siwn-replay-cache-in-memory-default.md). |

## Port-vs-git-dep

The crate carries an in-tree port of Nockchain's math primitives (Belt / F6 / Cheetah / Tip5) under `src/math/` rather than depending on `nockchain-math` directly. Rationale and parity-test strategy are recorded in [ADR-0007](../../docs/decisions/0007-phase3-inline-port.md).

## Usage

Sign an `Authorization`:

```rust no_run
# async fn run() {
use ibig::UBig;
use x402_client::Signer;
use x402_nockchain_crypto::{NockchainSigner, SchnorrPrivateKey};
use x402_types::payment::{Authorization, NoteName, NoteRef, PaymentRequirements};

let sk = SchnorrPrivateKey::new(UBig::from(123_456_789u64)).unwrap();
let signer = NockchainSigner::new(sk).unwrap();

let auth = Authorization {
    from: signer.from_identifier(),
    to: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".into(),
    value: "65536".into(),
    fee: "10".into(),
    nonce: "test-nonce".into(),
    valid_after: 0,
    valid_before: 4_000_000_000,
    notes: vec![NoteRef {
        name: NoteName { first: "first".into(), last: "last".into() },
        assets: "65536".into(),
        lock: Default::default(),
    }],
    change_address: signer.from_identifier(),
};

let requirements = PaymentRequirements {
    scheme: "exact".into(),
    network: "nockchain:mainnet".into(),
    max_amount_required: "65536".into(),
    resource: "https://example.test/r".into(),
    asset: "NOCK".into(),
    pay_to: auth.to.clone(),
    max_timeout_seconds: 60,
    description: None,
    mime_type: None,
    output_schema: None,
    extra: None,
    extensions: None,
};

let sig = signer.sign_authorization(&auth, &requirements).await.unwrap();
assert_eq!(sig.pubkey.is_empty(), false);
# }
```

## Golden-vector parity

`tests/golden.rs` exercises Cheetah scalar multiplication + Tip5 hashing against vectors captured from `nockchain-wallet`. Any divergence between the in-tree port and the upstream Hoon kernels surfaces here. The Tip5 sponge in `x402_sign_message_*` is the canonical signing input — clients that produce a different digest will fail facilitator verification regardless of the underlying signature scheme.
