//! Emit the pinned Phase-3 golden vector to stdout. Run with:
//!
//!     cargo run -p x402-nockchain-crypto --example gen_golden > \
//!         crates/x402-nockchain-crypto/tests/fixtures/golden_1.json
//!
//! Regenerate this whenever the spec §5.4.1 signing path or the
//! in-tree math port changes. The regression test
//! `signer_output_matches_pinned_fixture` re-runs the signer and
//! byte-compares against the checked-in fixture.

use ibig::UBig;
use serde_json::json;
use x402_nockchain_crypto::{
    pkh_belts_to_base58, schnorr::SchnorrPrivateKey, NockchainSigner,
};
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{Authorization, NoteName, NoteRef, PaymentRequirements};
use x402_client::Signer;

#[tokio::main]
async fn main() {
    use x402_nockchain_crypto::prelude::Belt;
    let sk = SchnorrPrivateKey::new(UBig::from(123_456_789u64)).unwrap();
    let signer = NockchainSigner::new(sk).unwrap();
    // Deterministic base58 stand-ins for PKHs + note names. These are
    // NOT real UTXOs; the golden vector regresses the signing path, not
    // on-chain validity.
    let to_pkh = pkh_belts_to_base58(&[
        Belt(11), Belt(22), Belt(33), Belt(44), Belt(55),
    ]);
    let nonce = pkh_belts_to_base58(&[
        Belt(10_000), Belt(20_000), Belt(30_000), Belt(40_000), Belt(50_000),
    ]);
    let note_first = pkh_belts_to_base58(&[
        Belt(1), Belt(2), Belt(3), Belt(4), Belt(5),
    ]);
    let note_last = pkh_belts_to_base58(&[
        Belt(6), Belt(7), Belt(8), Belt(9), Belt(10),
    ]);
    let auth = Authorization {
        from: signer.from_identifier(),
        to: to_pkh,
        value: "65536".into(),
        fee: "10".into(),
        nonce,
        valid_after: 1_000_000,
        valid_before: 1_000_060,
        notes: vec![NoteRef {
            name: NoteName {
                first: note_first,
                last: note_last,
            },
            assets: "65536".into(),
            lock: Default::default(),
        }],
        change_address: signer.from_identifier(),
    };
    let requirements = PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        max_amount_required: "65536".into(),
        resource: "https://example.test/golden".into(),
        asset: "NOCK".into(),
        pay_to: auth.to.clone(),
        max_timeout_seconds: 60,
        description: None,
        mime_type: None,
        output_schema: None,
        extra: Some(json!({ "minFee": "10" })),
        extensions: None,
    };
    let signature = signer.sign_authorization(&auth, &requirements).await.unwrap();
    let exact = ExactNockchainPayload { signature, authorization: auth, signed_raw_tx: None };
    println!("{}", serde_json::to_string_pretty(&exact).unwrap());
}
