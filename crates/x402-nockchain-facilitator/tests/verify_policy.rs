//! R1.1 verifier-policy hardening tests for the four checks `verify_envelope`
//! gained per `06-facilitator.md §6.6`.
//!
//! Each test pins a single rejection variant via the typed [`VerifyError`]
//! enum. The clock is driven through [`MockClock`] so time-window
//! boundaries are deterministic; the replay cache is the in-memory
//! default; the cap and asset checks are exercised by mutating the
//! fixture inputs after signing (so the signature still verifies).

use std::sync::Arc;
use std::time::Duration;

use ibig::UBig;
use serde_json::{json, Value};
use x402_client::Signer;
use x402_nockchain_crypto::prelude::Belt;
use x402_nockchain_crypto::{
    pkh_belts_to_base58, schnorr::SchnorrPrivateKey, NockchainSigner,
};
use x402_nockchain_facilitator::{
    in_memory_pool, verify_envelope, AppState, MockClock, VerifyError,
};
use x402_types::facilitator::VerifyRequest;
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{
    Authorization, NoteName, NoteRef, PaymentPayload, PaymentRequirements,
};

const NOW_FROZEN: u64 = 1_700_000_000;

fn signer() -> NockchainSigner {
    NockchainSigner::new(SchnorrPrivateKey::new(UBig::from(987_654_321u64)).unwrap()).unwrap()
}

/// Deterministic base58-valid 5-Belt PKH derived from a u64 tag.
fn b58_pkh(tag: u64) -> String {
    pkh_belts_to_base58(&[
        Belt(tag),
        Belt(tag.wrapping_add(1)),
        Belt(tag.wrapping_add(2)),
        Belt(tag.wrapping_add(3)),
        Belt(tag.wrapping_add(4)),
    ])
}

fn requirements(pay_to: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:fakenet".into(),
        max_amount_required: "1000".into(),
        resource: "https://verify-policy.test/resource".into(),
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

/// Build an `Authorization` whose `valid_after` / `valid_before` and
/// `nonce` are caller-controlled, sign it under the supplied signer, and
/// wrap into a [`VerifyRequest`] with `requirements`.
///
/// `nonce_seed` is a u64 tag turned into a base58-valid 5-Belt nonce —
/// `Authorization.nonce` is decoded back to bytes inside the §5.4.1
/// digest, so plain ASCII strings would fail the signer's base58 parse.
async fn signed_request(
    signer: &NockchainSigner,
    requirements: &PaymentRequirements,
    valid_after: u64,
    valid_before: u64,
    value: &str,
    nonce_seed: u64,
) -> VerifyRequest {
    let auth = Authorization {
        from: signer.from_identifier(),
        to: requirements.pay_to.clone(),
        value: value.into(),
        fee: "10".into(),
        nonce: b58_pkh(nonce_seed),
        valid_after,
        valid_before,
        notes: vec![NoteRef {
            name: NoteName {
                first: b58_pkh(nonce_seed.wrapping_add(1_000)),
                last: b58_pkh(nonce_seed.wrapping_add(2_000)),
            },
            assets: value.into(),
            lock: Default::default(),
        }],
        change_address: signer.from_identifier(),
    };
    let signature = signer
        .sign_authorization(&auth, requirements)
        .await
        .expect("sign");
    let exact = ExactNockchainPayload {
        signature,
        authorization: auth,
        signed_raw_tx: None,
    };
    let payload = PaymentPayload {
        x402_version: 2,
        scheme: requirements.scheme.clone(),
        network: requirements.network.clone(),
        payload: serde_json::to_value(&exact).unwrap(),
        extensions: None,
    };
    VerifyRequest {
        payload,
        requirements: requirements.clone(),
    }
}

async fn test_state(clock: MockClock) -> AppState {
    let pool = in_memory_pool().await.expect("pool");
    AppState::with_stub_chain_sqlite(pool)
        .with_clock(Arc::new(clock))
        .with_clock_skew_tolerance(Duration::from_secs(5))
        .with_replay_ttl(Duration::from_secs(600))
}

// ---------------------------------------------------------------------------
// 1. Time window
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_authorization_before_valid_after() {
    // Window opens at NOW_FROZEN + 100; the frozen clock is at
    // NOW_FROZEN, well below the open edge even after a 5s skew.
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    let req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN + 100,
        NOW_FROZEN + 200,
        "1000",
        100,
    )
    .await;

    let outcome = verify_envelope(&state, &req).await;
    match outcome {
        Err(VerifyError::OutsideTimeWindow {
            now,
            valid_after,
            valid_before,
        }) => {
            assert_eq!(now, NOW_FROZEN);
            assert_eq!(valid_after, NOW_FROZEN + 100);
            assert_eq!(valid_before, NOW_FROZEN + 200);
        }
        other => panic!("expected OutsideTimeWindow, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_authorization_after_valid_before() {
    // Window closed at NOW_FROZEN - 100; frozen clock at NOW_FROZEN
    // is past the close edge even with 5s skew.
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    let req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 200,
        NOW_FROZEN - 100,
        "1000",
        200,
    )
    .await;

    let outcome = verify_envelope(&state, &req).await;
    assert!(
        matches!(outcome, Err(VerifyError::OutsideTimeWindow { .. })),
        "expected OutsideTimeWindow, got {outcome:?}"
    );
}

#[tokio::test]
async fn accepts_authorization_within_skew_tolerance() {
    // Window opens 3s in the future, but skew tolerance is 5s, so
    // upper edge = now + 5 >= valid_after. Should accept.
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    let req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN + 3,
        NOW_FROZEN + 60,
        "1000",
        300,
    )
    .await;

    verify_envelope(&state, &req).await.expect("inside skew tolerance");
}

// ---------------------------------------------------------------------------
// 2. Replay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_replayed_nonce() {
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    let req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 5,
        NOW_FROZEN + 60,
        "1000",
        400,
    )
    .await;
    let expected_nonce = b58_pkh(400);

    // First call records the nonce.
    verify_envelope(&state, &req).await.expect("first call accepts");
    // Second call rejects.
    let outcome = verify_envelope(&state, &req).await;
    match outcome {
        Err(VerifyError::NonceReplayed { nonce }) => {
            assert_eq!(nonce, expected_nonce);
        }
        other => panic!("expected NonceReplayed, got {other:?}"),
    }
}

#[tokio::test]
async fn distinct_nonces_do_not_collide() {
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    let req_pay = requirements(&signer.from_identifier());

    let r1 = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 5,
        NOW_FROZEN + 60,
        "1000",
        500,
    )
    .await;
    let r2 = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 5,
        NOW_FROZEN + 60,
        "1000",
        600,
    )
    .await;

    verify_envelope(&state, &r1).await.expect("first nonce accepts");
    verify_envelope(&state, &r2).await.expect("second distinct nonce accepts");
}

// ---------------------------------------------------------------------------
// 3. Over max amount
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_value_over_max_amount() {
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    // requirements caps max at "1000"; auth.value = "5000" — over.
    let req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 5,
        NOW_FROZEN + 60,
        "5000",
        700,
    )
    .await;

    let outcome = verify_envelope(&state, &req).await;
    match outcome {
        Err(VerifyError::OverMaxAmount {
            value,
            max_amount_required,
        }) => {
            assert_eq!(value, "5000");
            assert_eq!(max_amount_required, "1000");
        }
        other => panic!("expected OverMaxAmount, got {other:?}"),
    }
}

#[tokio::test]
async fn accepts_value_at_max_amount() {
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    let req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 5,
        NOW_FROZEN + 60,
        "1000",
        800,
    )
    .await;
    verify_envelope(&state, &req).await.expect("value == cap accepts");
}

// ---------------------------------------------------------------------------
// 4. Asset mismatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_asset_mismatch() {
    let clock = MockClock::frozen_at(NOW_FROZEN);
    let state = test_state(clock).await;
    let signer = signer();
    // Mutate `requirements.asset` after signing — `auth` is unchanged
    // so the signature still verifies, and the asset check fires.
    let mut req_pay = requirements(&signer.from_identifier());
    let req = signed_request(
        &signer,
        &req_pay,
        NOW_FROZEN - 5,
        NOW_FROZEN + 60,
        "1000",
        900,
    )
    .await;
    req_pay.asset = "USDC".into();
    let mut req = req;
    req.requirements.asset = req_pay.asset.clone();

    // The payload's network is `nockchain:fakenet`; the implicit asset
    // is `NOCK`. `requirements.asset = "USDC"` triggers the mismatch.
    let outcome = verify_envelope(&state, &req).await;
    match outcome {
        Err(VerifyError::AssetMismatch { expected, actual }) => {
            assert_eq!(expected, "NOCK");
            assert_eq!(actual, "USDC");
        }
        other => panic!("expected AssetMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Conversion: VerifyError -> FacilitatorError preserves codes
// ---------------------------------------------------------------------------

#[test]
fn verify_error_maps_to_facilitator_codes() {
    use x402_types::facilitator::FacilitatorError;
    let cases: Vec<(VerifyError, &str)> = vec![
        (
            VerifyError::OutsideTimeWindow {
                now: 1,
                valid_after: 2,
                valid_before: 3,
            },
            "outside_time_window",
        ),
        (
            VerifyError::NonceReplayed { nonce: "n".into() },
            "nonce_replayed",
        ),
        (
            VerifyError::OverMaxAmount {
                value: "5".into(),
                max_amount_required: "3".into(),
            },
            "over_max_amount",
        ),
        (
            VerifyError::AssetMismatch {
                expected: "NOCK".into(),
                actual: "USDC".into(),
            },
            "asset_mismatch",
        ),
        (
            VerifyError::SpecCoded {
                code: "invalid_signature".into(),
                message: "x".into(),
            },
            "invalid_signature",
        ),
    ];
    for (err, expected_code) in cases {
        let label = err.rejection_label().to_string();
        let mapped: FacilitatorError = err.into();
        assert_eq!(mapped.code, expected_code);
        assert_eq!(label, expected_code);
    }
}

// Suppress dead-code for the unused JSON Value import on macro-expanded paths.
#[allow(dead_code)]
fn _unused() -> Value {
    json!(null)
}
