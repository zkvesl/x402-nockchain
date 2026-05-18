//! R1.2 scheme-registry contract tests.
//!
//! Pins (a) the registry's lookup behavior, (b) the `UnknownScheme`
//! rejection that replaces pre-R1.2's silent-pass for unknown
//! `(scheme, network)` pairs (per ADR-0020), and (c) the default
//! registry's two-handler set (`exact` + `upto` × `nockchain:*`).

use std::sync::Arc;
use std::time::Duration;

use ibig::UBig;
use serde_json::json;
use x402_client::Signer;
use x402_nockchain_crypto::prelude::Belt;
use x402_nockchain_crypto::{
    pkh_belts_to_base58, schnorr::SchnorrPrivateKey, NockchainSigner,
};
use x402_nockchain_facilitator::{
    in_memory_pool, verify_envelope, AppState, MockClock, SchemeHandlerRegistry, VerifyError,
};
use x402_types::facilitator::VerifyRequest;
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{
    Authorization, NoteName, NoteRef, PaymentPayload, PaymentRequirements,
};

const NOW: u64 = 1_700_000_000;

fn signer() -> NockchainSigner {
    NockchainSigner::new(SchnorrPrivateKey::new(UBig::from(11_223_344u64)).unwrap()).unwrap()
}

fn b58(tag: u64) -> String {
    pkh_belts_to_base58(&[
        Belt(tag),
        Belt(tag.wrapping_add(1)),
        Belt(tag.wrapping_add(2)),
        Belt(tag.wrapping_add(3)),
        Belt(tag.wrapping_add(4)),
    ])
}

fn requirements(scheme: &str, network: &str, pay_to: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: scheme.into(),
        network: network.into(),
        max_amount_required: "1000".into(),
        resource: "https://scheme-registry.test/r".into(),
        asset: "NOCK".into(),
        pay_to: pay_to.into(),
        max_timeout_seconds: 60,
        description: None,
        mime_type: None,
        output_schema: None,
        extra: Some(json!({ "minFee": "10" })),
        extensions: None,
    }
}

async fn signed_request(
    signer: &NockchainSigner,
    req: &PaymentRequirements,
    nonce_seed: u64,
    value: &str,
) -> VerifyRequest {
    let auth = Authorization {
        from: signer.from_identifier(),
        to: req.pay_to.clone(),
        value: value.into(),
        fee: "10".into(),
        nonce: b58(nonce_seed),
        valid_after: NOW.saturating_sub(5),
        valid_before: NOW.saturating_add(60),
        notes: vec![NoteRef {
            name: NoteName {
                first: b58(nonce_seed.wrapping_add(1_000)),
                last: b58(nonce_seed.wrapping_add(2_000)),
            },
            assets: value.into(),
            lock: Default::default(),
        }],
        change_address: signer.from_identifier(),
    };
    let signature = signer.sign_authorization(&auth, req).await.expect("sign");
    let exact = ExactNockchainPayload {
        signature,
        authorization: auth,
        signed_raw_tx: None,
    };
    VerifyRequest {
        payload: PaymentPayload {
            x402_version: 2,
            scheme: req.scheme.clone(),
            network: req.network.clone(),
            payload: serde_json::to_value(&exact).unwrap(),
            extensions: None,
        },
        requirements: req.clone(),
    }
}

async fn test_state() -> AppState {
    let pool = in_memory_pool().await.expect("pool");
    AppState::with_stub_chain_sqlite(pool)
        .with_clock(Arc::new(MockClock::frozen_at(NOW)))
        .with_clock_skew_tolerance(Duration::from_secs(5))
        .with_replay_ttl(Duration::from_secs(600))
}

// ---------------------------------------------------------------------------
// Registry lookup contract
// ---------------------------------------------------------------------------

#[test]
fn empty_registry_resolves_nothing() {
    let registry = SchemeHandlerRegistry::new();
    assert!(registry.is_empty());
    assert!(registry.handler_for("exact", "nockchain:fakenet").is_none());
    assert!(registry.handler_for("upto", "nockchain:mainnet").is_none());
}

#[test]
fn default_registry_carries_exact_and_upto() {
    let registry = SchemeHandlerRegistry::with_default_handlers();
    assert_eq!(registry.len(), 2);
    let exact = registry
        .handler_for("exact", "nockchain:fakenet")
        .expect("exact handler registered");
    assert_eq!(exact.scheme(), "exact");
    let upto = registry
        .handler_for("upto", "nockchain:mainnet")
        .expect("upto handler registered");
    assert_eq!(upto.scheme(), "upto");
}

#[test]
fn registry_rejects_unknown_scheme_pair() {
    let registry = SchemeHandlerRegistry::with_default_handlers();
    assert!(
        registry.handler_for("magic", "nockchain:fakenet").is_none(),
        "unknown scheme",
    );
    assert!(
        registry.handler_for("exact", "ethereum:mainnet").is_none(),
        "unknown network prefix",
    );
}

#[test]
fn registry_matches_network_by_prefix() {
    let registry = SchemeHandlerRegistry::with_default_handlers();
    for net in [
        "nockchain:fakenet",
        "nockchain:mainnet",
        "nockchain:testnet-2026",
        "nockchain:",
    ] {
        assert!(
            registry.handler_for("exact", net).is_some(),
            "exact handler should match `{net}`",
        );
    }
}

// ---------------------------------------------------------------------------
// verify_envelope dispatch behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_scheme_rejects_via_verify_envelope() {
    let signer = signer();
    let req_pay = requirements("magic", "nockchain:fakenet", &signer.from_identifier());
    let req = signed_request(&signer, &req_pay, 50, "1000").await;
    let state = test_state().await;

    let outcome = verify_envelope(&state, &req).await;
    match outcome {
        Err(VerifyError::UnknownScheme { scheme, network }) => {
            assert_eq!(scheme, "magic");
            assert_eq!(network, "nockchain:fakenet");
        }
        other => panic!("expected UnknownScheme, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_network_rejects_via_verify_envelope() {
    let signer = signer();
    let req_pay = requirements("exact", "ethereum:mainnet", &signer.from_identifier());
    let req = signed_request(&signer, &req_pay, 60, "1000").await;
    let state = test_state().await;

    let outcome = verify_envelope(&state, &req).await;
    assert!(
        matches!(outcome, Err(VerifyError::UnknownScheme { .. })),
        "expected UnknownScheme, got {outcome:?}"
    );
}

#[tokio::test]
async fn exact_handler_preserves_pre_r1_2_accept_path() {
    // Concrete regression: the exact handler running through the
    // registry must accept the same `(exact, nockchain:fakenet)` flow
    // R1.1 closed on. Catches a future drift in the lift.
    let signer = signer();
    let req_pay = requirements("exact", "nockchain:fakenet", &signer.from_identifier());
    let req = signed_request(&signer, &req_pay, 70, "1000").await;
    let state = test_state().await;

    verify_envelope(&state, &req)
        .await
        .expect("exact handler accepts the R1.1 happy-path payload");
}
