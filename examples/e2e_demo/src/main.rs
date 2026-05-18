//! End-to-end demo: stub resource server + real `x402-nockchain-facilitator`
//! + `X402Client` with a real Phase-3 `NockchainSigner`.
//!
//! Round trip:
//!   1. Spawn the real facilitator (`x402-nockchain-facilitator::router`)
//!      with an in-memory SQLite pool, on 127.0.0.1:0.
//!   2. Spawn a tiny resource server on 127.0.0.1:0 that returns 402 with
//!      a PaymentRequired envelope containing an MCP `bazaar` extension.
//!   3. Drive the full flow: GET resource -> 402; build `PaymentPayload`
//!      with `NockchainSigner`; POST facilitator `/verify`; assert 200 +
//!      EXTENSION-RESPONSES indicates the bazaar cataloging succeeded.
//!   4. Query `GET /discovery/resources?type=mcp&limit=10&offset=0` and
//!      confirm the resource appears.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use ibig::UBig;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;
use x402_client::{BazaarClient, X402Client};
use x402_nockchain_crypto::{NockchainSigner, SchnorrPrivateKey};
use x402_nockchain_facilitator::{in_memory_pool, router as facilitator_router, AppState};
use x402_types::bazaar::BazaarExtension;
use x402_types::payment::{PaymentRequired, PaymentRequirements, PaymentResource};
use x402_types::{ListDiscoveryResourcesParams, McpTransport};

const X402_VERSION: u32 = 2;
const RESOURCE_URL_PATH: &str = "/mcp/echo";

#[tokio::main]
async fn main() -> Result<()> {
    // --- Facilitator ------------------------------------------------------
    let pool = in_memory_pool().await.context("boot sqlite")?;
    let facilitator = facilitator_router(AppState::with_stub_chain_sqlite(pool));
    let facilitator_listener = TcpListener::bind("127.0.0.1:0").await?;
    let facilitator_addr = facilitator_listener.local_addr()?;
    let facilitator_base = format!("http://{}", facilitator_addr);
    tokio::spawn(async move {
        let _ = axum::serve(facilitator_listener, facilitator).await;
    });
    println!("[demo] facilitator listening on {}", facilitator_base);

    // --- Resource server --------------------------------------------------
    let advertisement = build_mcp_advertisement();
    let resource_listener = TcpListener::bind("127.0.0.1:0").await?;
    let resource_addr = resource_listener.local_addr()?;
    let resource_base = format!("http://{}", resource_addr);
    let resource_url = format!("{}{}", resource_base, RESOURCE_URL_PATH);

    let requirements = default_requirements(&resource_url);
    let advertisement = Arc::new(advertisement);
    let resource_state = ResourceState {
        requirements: Arc::new(requirements.clone()),
        bazaar: advertisement.clone(),
    };
    let resource_app = Router::new()
        .route(RESOURCE_URL_PATH, get(resource_handler))
        .with_state(resource_state);
    tokio::spawn(async move {
        let _ = axum::serve(resource_listener, resource_app).await;
    });
    println!("[demo] resource server listening on {}", resource_base);

    // --- Client -----------------------------------------------------------
    let client = X402Client::new();
    let signer = NockchainSigner::new(
        SchnorrPrivateKey::new(UBig::from(123_456_789u64)).expect("demo key"),
    )
    .expect("construct demo signer");

    let envelope = client
        .get_payment_required(&resource_url)
        .await
        .context("initial GET should return 402")?;
    println!(
        "[demo] 402 body:\n{}",
        serde_json::to_string_pretty(&envelope)?
    );
    assert_eq!(envelope.accepts.len(), 1);

    let requirements = envelope.accepts[0].clone();
    let payload = x402_client::build_exact_nockchain_payload(
        &requirements,
        &signer,
        &envelope.extensions,
    )
    .await?;

    let (verify, ext_header) = client
        .verify(&facilitator_base, &payload, &requirements)
        .await?;
    println!("[demo] verify response: {:?}", verify);
    println!("[demo] EXTENSION-RESPONSES header decoded: {:?}", ext_header);

    assert!(verify.valid, "facilitator rejected signed payload");
    let ext = ext_header.expect("EXTENSION-RESPONSES header present");
    let bazaar_outcome = ext.bazaar.expect("bazaar key present");
    assert!(
        matches!(
            bazaar_outcome.status,
            x402_types::payment::BazaarExtensionStatus::Success
        ),
        "bazaar cataloging did not succeed: {:?}",
        bazaar_outcome
    );

    // --- Discovery query --------------------------------------------------
    let bazaar = BazaarClient::new(facilitator_base.clone());
    let listing = bazaar
        .list_resources(ListDiscoveryResourcesParams {
            kind: Some("mcp".into()),
            limit: Some(10),
            offset: Some(0),
            ..Default::default()
        })
        .await?;
    println!(
        "[demo] discovery query returned {} item(s):\n{}",
        listing.items.len(),
        serde_json::to_string_pretty(&listing)?
    );

    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.pagination.total, 1);
    assert_eq!(listing.items[0].kind, "mcp");
    assert_eq!(listing.items[0].resource, resource_url);

    println!("[demo] OK — phase-3 signed round trip succeeded");
    Ok(())
}

// ---------------------------------------------------------------------------
// Resource server (stand-in for a paid API)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ResourceState {
    requirements: Arc<PaymentRequirements>,
    bazaar: Arc<BazaarExtension>,
}

async fn resource_handler(State(state): State<ResourceState>) -> impl IntoResponse {
    let envelope = PaymentRequired {
        x402_version: X402_VERSION,
        error: "Payment required".into(),
        resource: PaymentResource {
            url: state.requirements.resource.clone(),
            description: Some("Demo MCP echo tool".into()),
            mime_type: Some("application/json".into()),
        },
        accepts: vec![(*state.requirements).clone()],
        extensions: Some(
            [(
                "bazaar".to_string(),
                serde_json::to_value(&*state.bazaar).unwrap_or(json!(null)),
            )]
            .into_iter()
            .collect(),
        ),
    };
    (StatusCode::PAYMENT_REQUIRED, Json(envelope))
}

fn build_mcp_advertisement() -> BazaarExtension {
    x402_advertiser::declare_mcp(
        "echo",
        Some("Echo the input string back as output. Demo tool.".into()),
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "Text to echo back" }
            },
            "required": ["message"]
        }),
        Some(McpTransport::StreamableHttp),
        Some(json!({ "message": "hello, nockchain" })),
        None,
    )
}

fn default_requirements(resource_url: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        max_amount_required: "65536".into(),
        resource: resource_url.to_string(),
        asset: "NOCK".into(),
        pay_to: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".into(),
        max_timeout_seconds: 60,
        description: Some("Demo MCP echo tool".into()),
        mime_type: Some("application/json".into()),
        output_schema: None,
        extra: Some(json!({ "minFee": "10" })),
        extensions: None,
    }
}
