//! Phase-3 three-service integration test with real Schnorr signatures.
//!
//! Spins up a resource server, the facilitator, and a client runner on
//! ephemeral ports, then drives the x402 round trip end-to-end with a
//! `NockchainSigner`. Also asserts the facilitator's negative path:
//! stub (all-zero) signatures MUST be rejected.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::{Duration as ChronoDuration, Utc};
use ibig::UBig;
use serde_json::json;
use tokio::net::TcpListener;
use x402_client::{build_exact_nockchain_payload, BazaarClient, StubSigner, X402Client};
use x402_nockchain_crypto::{
    NockchainSigner, SchnorrPrivateKey, SiwnParams, SiwnSigner,
};
use x402_nockchain_facilitator::{
    in_memory_pool, router as facilitator_router, router_with_siwn, AppState, SiwnGate,
};
use x402_types::bazaar::BazaarExtension;
use x402_types::payment::{
    BazaarExtensionStatus, PaymentRequired, PaymentRequirements, PaymentResource,
};
use x402_types::{ListDiscoveryResourcesParams, McpTransport};

// ---------------------------------------------------------------------------
// Fixture helpers (trimmed clone of phase-2 e2e_stub scaffolding)
// ---------------------------------------------------------------------------

const RESOURCE_PATH: &str = "/mcp/echo";

async fn spawn_facilitator() -> Result<(String, AppState)> {
    let pool = in_memory_pool().await?;
    let state = AppState::with_stub_chain_sqlite(pool);
    let app = facilitator_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{}", addr), state))
}

async fn spawn_facilitator_with_siwn(domain: &str) -> Result<(String, AppState, SiwnGate)> {
    let pool = in_memory_pool().await?;
    let state = AppState::with_stub_chain_sqlite(pool);
    let gate = SiwnGate::new(domain);
    let app = router_with_siwn(state.clone(), gate.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{}", addr), state, gate))
}

#[derive(Clone)]
struct ResourceState {
    requirements: Arc<PaymentRequirements>,
    bazaar: Arc<BazaarExtension>,
}

async fn resource_handler(State(state): State<ResourceState>) -> impl IntoResponse {
    let envelope = PaymentRequired {
        x402_version: 2,
        error: "Payment required".into(),
        resource: PaymentResource {
            url: state.requirements.resource.clone(),
            description: Some("MCP echo demo".into()),
            mime_type: Some("application/json".into()),
        },
        accepts: vec![(*state.requirements).clone()],
        extensions: Some(
            [(
                "bazaar".to_string(),
                serde_json::to_value(&*state.bazaar).unwrap(),
            )]
            .into_iter()
            .collect(),
        ),
    };
    (StatusCode::PAYMENT_REQUIRED, Json(envelope))
}

async fn spawn_resource_server(bazaar: BazaarExtension) -> Result<(String, PaymentRequirements)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("http://{}", addr);
    let resource_url = format!("{}{}", base, RESOURCE_PATH);

    let requirements = PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        max_amount_required: "65536".into(),
        resource: resource_url.clone(),
        asset: "NOCK".into(),
        pay_to: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".into(),
        max_timeout_seconds: 60,
        description: Some("Echo".into()),
        mime_type: Some("application/json".into()),
        output_schema: None,
        extra: Some(json!({ "minFee": "10" })),
        extensions: None,
    };

    let state = ResourceState {
        requirements: Arc::new(requirements.clone()),
        bazaar: Arc::new(bazaar),
    };
    let app = Router::new()
        .route(RESOURCE_PATH, get(resource_handler))
        .with_state(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((resource_url, requirements))
}

fn mcp_bazaar_extension() -> BazaarExtension {
    x402_advertiser::declare_mcp(
        "echo",
        Some("Echo tool".into()),
        json!({
            "type": "object",
            "properties": { "message": { "type": "string" } },
            "required": ["message"]
        }),
        Some(McpTransport::StreamableHttp),
        Some(json!({ "message": "hi" })),
        None,
    )
}

fn fresh_signer(seed: u64) -> NockchainSigner {
    NockchainSigner::new(SchnorrPrivateKey::new(UBig::from(seed)).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_service_round_trip_with_real_signer() -> Result<()> {
    let (facilitator_base, _state) = spawn_facilitator().await?;
    let (resource_url, _req) = spawn_resource_server(mcp_bazaar_extension()).await?;

    let client = X402Client::new();
    let signer = fresh_signer(123_456_789);

    // Step 1: GET → 402.
    let envelope = client.get_payment_required(&resource_url).await?;
    assert_eq!(envelope.accepts.len(), 1);

    // Step 2: build + sign a payload.
    let requirements = envelope.accepts[0].clone();
    let payload = build_exact_nockchain_payload(&requirements, &signer, &envelope.extensions)
        .await
        .context("build signed payload")?;

    // Step 3: POST /verify → valid + EXTENSION-RESPONSES present.
    let (verify, ext_header) = client.verify(&facilitator_base, &payload, &requirements).await?;
    assert!(verify.valid, "verify rejected real signature: {:?}", verify.error);
    let bazaar = ext_header
        .expect("EXTENSION-RESPONSES header missing")
        .bazaar
        .expect("bazaar key missing from EXTENSION-RESPONSES");
    assert!(matches!(bazaar.status, BazaarExtensionStatus::Success));

    // Step 4: discovery lists the cataloged resource.
    let listing = BazaarClient::new(facilitator_base.clone())
        .list_resources(ListDiscoveryResourcesParams {
            kind: Some("mcp".into()),
            limit: Some(10),
            offset: Some(0),
            ..Default::default()
        })
        .await?;
    assert_eq!(listing.pagination.total, 1);
    assert_eq!(listing.items[0].resource, resource_url);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facilitator_rejects_zero_signature() -> Result<()> {
    let (facilitator_base, _state) = spawn_facilitator().await?;
    let (resource_url, _req) = spawn_resource_server(mcp_bazaar_extension()).await?;

    let client = X402Client::new();
    let envelope = client.get_payment_required(&resource_url).await?;
    let requirements = envelope.accepts[0].clone();

    // StubSigner produces an all-zero SchnorrSignatureJson — the real
    // Phase-3 verifier MUST reject it.
    let payload =
        build_exact_nockchain_payload(&requirements, &StubSigner, &envelope.extensions).await?;
    let (verify, _) = client.verify(&facilitator_base, &payload, &requirements).await?;
    assert!(!verify.valid);
    assert_eq!(verify.error.as_ref().map(|e| e.code.as_str()), Some("invalid_signature"));

    // Catalog must stay empty — rejection cannot catalog.
    let listing = BazaarClient::new(facilitator_base)
        .list_resources(ListDiscoveryResourcesParams::default())
        .await?;
    assert_eq!(listing.pagination.total, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facilitator_rejects_tampered_authorization() -> Result<()> {
    let (facilitator_base, _state) = spawn_facilitator().await?;
    let (resource_url, _req) = spawn_resource_server(mcp_bazaar_extension()).await?;

    let signer = fresh_signer(111_222_333);
    let client = X402Client::new();
    let envelope = client.get_payment_required(&resource_url).await?;
    let requirements = envelope.accepts[0].clone();
    let mut payload =
        build_exact_nockchain_payload(&requirements, &signer, &envelope.extensions).await?;

    // Mutate the authorization after signing.
    let auth = payload.payload.get_mut("authorization").unwrap();
    auth["value"] = json!("99999");

    let (verify, _) = client.verify(&facilitator_base, &payload, &requirements).await?;
    assert!(!verify.valid);
    assert_eq!(verify.error.as_ref().map(|e| e.code.as_str()), Some("invalid_signature"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_responses_header_is_spec_base64() -> Result<()> {
    let (facilitator_base, _state) = spawn_facilitator().await?;
    let (resource_url, _req) = spawn_resource_server(mcp_bazaar_extension()).await?;

    let signer = fresh_signer(314_159_265);
    let envelope = X402Client::new().get_payment_required(&resource_url).await?;
    let requirements = envelope.accepts[0].clone();
    let payload =
        build_exact_nockchain_payload(&requirements, &signer, &envelope.extensions).await?;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/verify", facilitator_base))
        .json(&json!({ "payload": payload, "requirements": requirements }))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let raw_header = resp
        .headers()
        .get("extension-responses")
        .expect("EXTENSION-RESPONSES header missing")
        .clone();
    let decoded = B64
        .decode(raw_header.as_bytes())
        .context("EXTENSION-RESPONSES must be standard base64")?;
    let body: serde_json::Value =
        serde_json::from_slice(&decoded).context("decoded bytes must be valid JSON")?;
    assert_eq!(body["bazaar"]["status"], "success");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn siwn_middleware_rejects_unsigned_discovery() -> Result<()> {
    let (base, _state, _gate) = spawn_facilitator_with_siwn("facilitator.test").await?;
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/discovery/resources", base))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn siwn_middleware_accepts_signed_discovery() -> Result<()> {
    let (base, _state, _gate) = spawn_facilitator_with_siwn("facilitator.test").await?;
    let sk = SchnorrPrivateKey::new(UBig::from(42_424_242u64)).unwrap();
    let signer = SiwnSigner::new(sk.clone());
    let pk_b58 = sk.public_key().into_base58().unwrap();
    let now = Utc::now();
    let params = SiwnParams {
        domain: "facilitator.test".into(),
        address: pk_b58,
        uri: format!("{}/discovery/resources", base),
        version: "1".into(),
        chain_id: "nockchain:mainnet".into(),
        nonce: "siwn-nonce-1".into(),
        issued_at: now,
        expiration_time: now + ChronoDuration::minutes(5),
    };
    let header = signer.sign_header(&params)?;

    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/discovery/resources", base))
        .header("sign-in-with-x", header)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let listing: x402_types::DiscoveryResourcesResponse = resp.json().await?;
    assert_eq!(listing.pagination.total, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn siwn_middleware_rejects_replayed_nonce() -> Result<()> {
    let (base, _state, _gate) = spawn_facilitator_with_siwn("facilitator.test").await?;
    let sk = SchnorrPrivateKey::new(UBig::from(55_555_555u64)).unwrap();
    let signer = SiwnSigner::new(sk.clone());
    let pk_b58 = sk.public_key().into_base58().unwrap();
    let now = Utc::now();
    let params = SiwnParams {
        domain: "facilitator.test".into(),
        address: pk_b58,
        uri: format!("{}/discovery/resources", base),
        version: "1".into(),
        chain_id: "nockchain:mainnet".into(),
        nonce: "siwn-nonce-replay".into(),
        issued_at: now,
        expiration_time: now + ChronoDuration::minutes(5),
    };
    let header = signer.sign_header(&params)?;

    let http = reqwest::Client::new();
    let ok = http
        .get(format!("{}/discovery/resources", base))
        .header("sign-in-with-x", header.clone())
        .send()
        .await?;
    assert_eq!(ok.status(), StatusCode::OK);
    let replayed = http
        .get(format!("{}/discovery/resources", base))
        .header("sign-in-with-x", header)
        .send()
        .await?;
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}
