//! `PolicyEnforcedSigner` refusal tests.
//!
//! Verifies the R1.1 client-side policy seam: the adapter must
//! short-circuit before delegating to the inner signer when the policy
//! closure returns `Err(PolicyDenied)`, AND must pass the call through
//! when the policy returns `Ok`. The inner signer is a counter-bearing
//! fixture that records call sites — the test asserts both the surfaced
//! `Err` shape and the count of inner-signer invocations.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use x402_client::{PolicyDenied, PolicyEnforcedSigner, Signer};
use x402_types::payment::{
    Authorization, NoteName, NoteRef, PaymentRequirements, SchnorrSignatureJson,
};

/// Inner signer that records every `sign_authorization` invocation. Tests
/// assert the count to confirm the wrapper short-circuits without
/// calling through.
#[derive(Default)]
struct CountingSigner {
    calls: AtomicUsize,
}

impl CountingSigner {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Signer for CountingSigner {
    async fn sign_authorization(
        &self,
        _auth: &Authorization,
        _requirements: &PaymentRequirements,
    ) -> Result<SchnorrSignatureJson> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SchnorrSignatureJson::all_zero("counting-pubkey"))
    }

    fn from_identifier(&self) -> String {
        "counting-from".into()
    }
}

fn sample_authorization() -> Authorization {
    Authorization {
        from: "from-pkh".into(),
        to: "to-pkh".into(),
        value: "100".into(),
        fee: "1".into(),
        nonce: "nonce-1".into(),
        valid_after: 0,
        valid_before: u64::MAX,
        notes: vec![NoteRef {
            name: NoteName {
                first: "first".into(),
                last: "last".into(),
            },
            assets: "200".into(),
            lock: Default::default(),
        }],
        change_address: "from-pkh".into(),
    }
}

fn sample_requirements(max_amount: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:fakenet".into(),
        max_amount_required: max_amount.into(),
        resource: "/x".into(),
        asset: "NOCK".into(),
        pay_to: "to-pkh".into(),
        max_timeout_seconds: 60,
        description: None,
        mime_type: None,
        output_schema: None,
        extra: Some(json!({ "minFee": "1" })),
        extensions: None,
    }
}

#[tokio::test]
async fn policy_refusal_short_circuits_inner_signer() {
    let inner = CountingSigner::default();
    let inner_arc = Arc::new(inner);
    // Build the adapter from a clone of the Arc so the test still owns
    // a handle for asserting the call count.
    let signer = PolicyEnforcedSigner::from_arc(
        inner_arc.clone(),
        Arc::new(|_req: &PaymentRequirements| {
            Err(PolicyDenied::new("refused for test"))
        }),
    );

    let auth = sample_authorization();
    let req = sample_requirements("100");
    let outcome = signer.sign_authorization(&auth, &req).await;

    let err = outcome.expect_err("policy must refuse");
    assert!(
        err.to_string().contains("refused for test"),
        "error chain should surface the PolicyDenied reason: {err}"
    );
    assert_eq!(
        inner_arc.calls(),
        0,
        "inner signer must not be called when policy refuses"
    );
}

#[tokio::test]
async fn policy_pass_delegates_to_inner_signer() {
    let inner_arc = Arc::new(CountingSigner::default());
    let signer = PolicyEnforcedSigner::from_arc(
        inner_arc.clone(),
        Arc::new(|_req: &PaymentRequirements| Ok(())),
    );

    let auth = sample_authorization();
    let req = sample_requirements("100");
    let sig = signer
        .sign_authorization(&auth, &req)
        .await
        .expect("policy passes; inner signer runs");

    assert!(sig.is_all_zero(), "inner CountingSigner emits all-zero");
    assert_eq!(inner_arc.calls(), 1, "inner signer called exactly once");
}

#[tokio::test]
async fn policy_inspects_payment_requirements() {
    // Cap-style policy: refuse when `max_amount_required` is over 1_000.
    let inner_arc = Arc::new(CountingSigner::default());
    let signer = PolicyEnforcedSigner::from_arc(
        inner_arc.clone(),
        Arc::new(|req: &PaymentRequirements| {
            let max: u128 = req
                .max_amount_required
                .parse()
                .map_err(|_| PolicyDenied::new("max_amount_required not a u128"))?;
            if max > 1_000 {
                return Err(PolicyDenied::new(format!(
                    "max_amount_required {max} exceeds operator cap 1000"
                )));
            }
            Ok(())
        }),
    );

    let auth = sample_authorization();

    // Within the cap — passes.
    let req_ok = sample_requirements("500");
    signer
        .sign_authorization(&auth, &req_ok)
        .await
        .expect("under cap");
    assert_eq!(inner_arc.calls(), 1);

    // Over the cap — refused.
    let req_over = sample_requirements("9999");
    let err = signer
        .sign_authorization(&auth, &req_over)
        .await
        .expect_err("over cap must refuse");
    assert!(err.to_string().contains("operator cap"));
    assert_eq!(
        inner_arc.calls(),
        1,
        "no additional inner-signer call after refusal"
    );
}

#[test]
fn policy_denied_message_carries_reason() {
    let denied = PolicyDenied::new("specific reason");
    assert!(denied.to_string().contains("specific reason"));
}
