//! Golden-vector tests for the Phase-3+4 signing path.
//!
//! Self-generated per ADR-0007: the signer runs on a fixed
//! `(private key, authorization)` pair and the output is pinned as a
//! JSON fixture. Any drift in the math port or the spec §5.4.1
//! canonicalisation breaks these tests immediately.
//!
//! Chain parity (against `nockchain-math` values) lives in the math
//! module's own tests. The Hoon-side golden vector bridge is scheduled
//! for Phase 5 (ADR-0013).

use std::path::PathBuf;

use ibig::UBig;
use serde_json::{json, Value};
use x402_client::Signer;
use x402_nockchain_crypto::prelude::Belt;
use x402_nockchain_crypto::schnorr::SchnorrPrivateKey;
use x402_nockchain_crypto::{pkh_belts_to_base58, NockchainSigner, NockchainVerifier};
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{Authorization, NoteName, NoteRef, PaymentPayload, PaymentRequirements};

fn signer_from_seed(seed: u64) -> NockchainSigner {
    NockchainSigner::new(SchnorrPrivateKey::new(UBig::from(seed)).unwrap()).unwrap()
}

/// Deterministic base58-valid PKH built from a seed seed.
fn stub_pkh(tag: u64) -> String {
    pkh_belts_to_base58(&[
        Belt(tag),
        Belt(tag + 1),
        Belt(tag + 2),
        Belt(tag + 3),
        Belt(tag + 4),
    ])
}

fn sample_authorization(from_pkh: &str, to_pkh: &str) -> Authorization {
    Authorization {
        from: from_pkh.to_string(),
        to: to_pkh.to_string(),
        value: "65536".into(),
        fee: "10".into(),
        nonce: stub_pkh(10_000),
        valid_after: 1_000_000,
        valid_before: 1_000_060,
        notes: vec![NoteRef {
            name: NoteName {
                first: stub_pkh(1),
                last: stub_pkh(6),
            },
            assets: "65536".into(),
            lock: Default::default(),
        }],
        change_address: from_pkh.to_string(),
    }
}

async fn build_signed_payload(
    signer: &NockchainSigner,
    auth: &Authorization,
) -> (ExactNockchainPayload, PaymentPayload<Value>) {
    // The signer ignores `requirements`; a synthetic value keeps the
    // call site free of caller-provided test data while exercising the
    // full trait surface introduced in R1.1.
    let requirements = sample_requirements(&auth.to, "https://example.test/golden");
    let signature = signer.sign_authorization(auth, &requirements).await.unwrap();
    let exact = ExactNockchainPayload {
        signature,
        authorization: auth.clone(),
        signed_raw_tx: None,
    };
    let value = serde_json::to_value(&exact).unwrap();
    let envelope = PaymentPayload {
        x402_version: 2,
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        payload: value,
        extensions: None,
    };
    (exact, envelope)
}

fn sample_requirements(pay_to: &str, resource_url: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        max_amount_required: "65536".into(),
        resource: resource_url.to_string(),
        asset: "NOCK".into(),
        pay_to: pay_to.to_string(),
        max_timeout_seconds: 60,
        description: None,
        mime_type: None,
        output_schema: None,
        extra: Some(json!({ "minFee": "10" })),
        extensions: None,
    }
}

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

#[tokio::test]
async fn signer_output_matches_pinned_fixture() {
    let signer = signer_from_seed(123_456_789);
    let auth = Authorization {
        from: signer.from_identifier(),
        to: pkh_belts_to_base58(&[Belt(11), Belt(22), Belt(33), Belt(44), Belt(55)]),
        value: "65536".into(),
        fee: "10".into(),
        nonce: pkh_belts_to_base58(&[
            Belt(10_000),
            Belt(20_000),
            Belt(30_000),
            Belt(40_000),
            Belt(50_000),
        ]),
        valid_after: 1_000_000,
        valid_before: 1_000_060,
        notes: vec![NoteRef {
            name: NoteName {
                first: pkh_belts_to_base58(&[Belt(1), Belt(2), Belt(3), Belt(4), Belt(5)]),
                last: pkh_belts_to_base58(&[Belt(6), Belt(7), Belt(8), Belt(9), Belt(10)]),
            },
            assets: "65536".into(),
            lock: Default::default(),
        }],
        change_address: signer.from_identifier(),
    };
    let (exact, _) = build_signed_payload(&signer, &auth).await;

    let actual = serde_json::to_value(&exact).unwrap();
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path("golden_1.json")).unwrap())
            .unwrap();
    assert_eq!(actual, expected, "signature drifted from pinned fixture");
}

#[tokio::test]
async fn verifier_accepts_signed_payload() {
    let signer = signer_from_seed(987_654_321);
    let pay_to = stub_pkh(200);
    let auth = sample_authorization(&signer.from_identifier(), &pay_to);
    let (_, payload) = build_signed_payload(&signer, &auth).await;
    let req = sample_requirements(&pay_to, "https://example.test/resource");

    NockchainVerifier::new()
        .verify_payment(&payload, &req)
        .expect("verifier accepts our own signature");
}

#[tokio::test]
async fn verifier_rejects_tampered_authorization() {
    let signer = signer_from_seed(111_222_333);
    let pay_to = stub_pkh(300);
    let auth = sample_authorization(&signer.from_identifier(), &pay_to);
    let (mut exact, _) = build_signed_payload(&signer, &auth).await;
    exact.authorization.value = "99999".into();
    let value = serde_json::to_value(&exact).unwrap();
    let payload: PaymentPayload<Value> = PaymentPayload {
        x402_version: 2,
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        payload: value,
        extensions: None,
    };
    let req = sample_requirements(&pay_to, "https://example.test/resource");
    assert!(NockchainVerifier::new().verify_payment(&payload, &req).is_err());
}

#[tokio::test]
async fn verifier_rejects_tampered_signature() {
    let signer = signer_from_seed(111_222_333);
    let pay_to = stub_pkh(400);
    let auth = sample_authorization(&signer.from_identifier(), &pay_to);
    let (_, mut payload) = build_signed_payload(&signer, &auth).await;
    let sig = payload
        .payload
        .get_mut("signature")
        .unwrap()
        .get_mut("schnorr")
        .unwrap()
        .get_mut("sig")
        .unwrap();
    sig[0] = json!("42");
    let req = sample_requirements(&pay_to, "https://example.test/resource");
    assert!(NockchainVerifier::new().verify_payment(&payload, &req).is_err());
}

#[tokio::test]
async fn different_keys_produce_different_signatures() {
    let from_pkh = stub_pkh(900);
    let to_pkh = stub_pkh(910);
    let auth = sample_authorization(&from_pkh, &to_pkh);
    let sig_a = {
        let signer = signer_from_seed(1);
        let (exact, _) = build_signed_payload(&signer, &auth).await;
        exact.signature
    };
    let sig_b = {
        let signer = signer_from_seed(2);
        let (exact, _) = build_signed_payload(&signer, &auth).await;
        exact.signature
    };
    assert_ne!(sig_a.pubkey, sig_b.pubkey);
    assert_ne!(sig_a.schnorr.chal, sig_b.schnorr.chal);
    assert_ne!(sig_a.schnorr.sig, sig_b.schnorr.sig);
}
