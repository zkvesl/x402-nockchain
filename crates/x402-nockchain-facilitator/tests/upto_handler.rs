//! R1.2 `(upto, nockchain:*)` handler tests.
//!
//! `upto` admits any `auth.value <= max_amount_required`. Pinned cases:
//!   - value far below cap accepts (the natural bounty-style use case),
//!   - value at exactly the cap accepts (boundary),
//!   - value above the cap rejects with `OverMaxAmount`.

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
    in_memory_pool, verify_envelope, AppState, MockClock, VerifyError,
};
use x402_types::facilitator::VerifyRequest;
use x402_types::nockchain::ExactNockchainPayload;
use x402_types::payment::{
    Authorization, NoteName, NoteRef, PaymentPayload, PaymentRequirements,
};

const NOW: u64 = 1_700_000_000;

fn signer() -> NockchainSigner {
    NockchainSigner::new(SchnorrPrivateKey::new(UBig::from(33_445_566u64)).unwrap()).unwrap()
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

fn upto_requirements(pay_to: &str, max_amount: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: "upto".into(),
        network: "nockchain:fakenet".into(),
        max_amount_required: max_amount.into(),
        resource: "https://upto-handler.test/resource".into(),
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

#[tokio::test]
async fn upto_accepts_value_below_cap() {
    let signer = signer();
    let req_pay = upto_requirements(&signer.from_identifier(), "1000");
    let req = signed_request(&signer, &req_pay, 100, "500").await;
    let state = test_state().await;
    verify_envelope(&state, &req)
        .await
        .expect("upto accepts value strictly below cap");
}

#[tokio::test]
async fn upto_accepts_value_at_cap_boundary() {
    let signer = signer();
    let req_pay = upto_requirements(&signer.from_identifier(), "1000");
    let req = signed_request(&signer, &req_pay, 200, "1000").await;
    let state = test_state().await;
    verify_envelope(&state, &req)
        .await
        .expect("upto accepts value equal to cap");
}

#[tokio::test]
async fn upto_accepts_zero_value() {
    // The bounty-style use case: a free-tier acknowledgement with
    // zero value. `exact` would require equality with the cap; `upto`
    // admits any value at or below the cap, including zero.
    let signer = signer();
    let req_pay = upto_requirements(&signer.from_identifier(), "1000");
    let req = signed_request(&signer, &req_pay, 300, "0").await;
    let state = test_state().await;
    verify_envelope(&state, &req)
        .await
        .expect("upto accepts zero value");
}

#[tokio::test]
async fn upto_rejects_value_above_cap() {
    let signer = signer();
    let req_pay = upto_requirements(&signer.from_identifier(), "1000");
    let req = signed_request(&signer, &req_pay, 400, "5000").await;
    let state = test_state().await;
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
async fn upto_rejects_asset_mismatch() {
    let signer = signer();
    let mut req_pay = upto_requirements(&signer.from_identifier(), "1000");
    let req = signed_request(&signer, &req_pay, 500, "200").await;
    req_pay.asset = "USDC".into();
    let mut req = req;
    req.requirements.asset = req_pay.asset.clone();
    let state = test_state().await;
    let outcome = verify_envelope(&state, &req).await;
    assert!(
        matches!(outcome, Err(VerifyError::AssetMismatch { .. })),
        "expected AssetMismatch, got {outcome:?}"
    );
}
