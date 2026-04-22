//! End-to-end demo: stub Nockchain x402 facilitator + advertiser + client.
//!
//! Round trip:
//!   1. Spawn a stub facilitator on 127.0.0.1:0 (axum). Exposes
//!      `POST /catalog` (cataloging shortcut — real facilitators catalog
//!      implicitly during /verify or /settle) and `GET /discovery/resources`
//!      (the upstream-canonical query API).
//!   2. Build a BazaarExtension for an MCP tool via `x402_advertiser::declare_mcp`.
//!   3. POST it to /catalog as if a 402-flow had just delivered it.
//!   4. Use BazaarClient to GET /discovery/resources and confirm the resource
//!      appears.

use anyhow::Result;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json as JsonResp,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use x402_client::BazaarClient;
use x402_types::{
    BazaarExtension, DiscoveryInput, DiscoveryResource, DiscoveryResourcesResponse,
    ListDiscoveryResourcesParams, McpTransport, Pagination,
};

const X402_VERSION: u32 = 2;

#[derive(Default)]
struct CatalogState {
    resources: Vec<DiscoveryResource>,
}

type Shared = Arc<Mutex<CatalogState>>;

#[derive(Deserialize)]
struct CatalogRequest {
    resource_url: String,
    extension: BazaarExtension,
}

async fn catalog(
    State(state): State<Shared>,
    Json(req): Json<CatalogRequest>,
) -> Result<JsonResp<serde_json::Value>, (StatusCode, String)> {
    if !req.extension.schema.is_object() {
        return Err((StatusCode::BAD_REQUEST, "schema must be an object".into()));
    }

    let kind = match &req.extension.info.input {
        DiscoveryInput::Http(_) => "http",
        DiscoveryInput::Mcp(_) => "mcp",
    };

    let resource = DiscoveryResource {
        resource: req.resource_url.clone(),
        kind: kind.to_string(),
        x402_version: X402_VERSION,
        accepts: vec![json!({
            "scheme": "exact",
            "network": "nockchain:mainnet",
            "maxAmountRequired": "65536",
        })],
        last_updated: chrono::Utc::now().to_rfc3339(),
        metadata: Some(serde_json::to_value(&req.extension.info).unwrap_or(json!(null))),
    };
    state.lock().unwrap().resources.push(resource);
    Ok(JsonResp(json!({ "bazaar": { "status": "success" } })))
}

async fn list_resources(
    State(state): State<Shared>,
    Query(params): Query<ListDiscoveryResourcesParams>,
) -> JsonResp<DiscoveryResourcesResponse> {
    let state = state.lock().unwrap();
    let mut filtered: Vec<DiscoveryResource> = state
        .resources
        .iter()
        .filter(|r| params.kind.as_deref().map_or(true, |k| r.kind == k))
        .cloned()
        .collect();

    let total = filtered.len() as u32;
    let offset = params.offset.unwrap_or(0) as usize;
    let limit = params.limit.unwrap_or(50) as usize;

    let items = if offset >= filtered.len() {
        Vec::new()
    } else {
        let end = (offset + limit).min(filtered.len());
        filtered.drain(offset..end).collect()
    };

    JsonResp(DiscoveryResourcesResponse {
        x402_version: X402_VERSION,
        items,
        pagination: Pagination {
            limit: params.limit.unwrap_or(50),
            offset: params.offset.unwrap_or(0),
            total,
        },
    })
}

async fn spawn_stub_facilitator() -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let state: Shared = Arc::new(Mutex::new(CatalogState::default()));
    let app = Router::new()
        .route("/catalog", post(catalog))
        .route("/discovery/resources", get(list_resources))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((addr, handle))
}

#[tokio::main]
async fn main() -> Result<()> {
    let (addr, _handle) = spawn_stub_facilitator().await?;
    let base = format!("http://{}", addr);
    println!("[demo] stub facilitator listening on {}", base);

    let mcp_ext = x402_advertiser::declare_mcp(
        "verify_intent_settlement",
        Some("Verify the proof attached to a Vesl intent settlement.".into()),
        json!({
            "type": "object",
            "properties": {
                "intentId": { "type": "string", "description": "Hex-encoded intent hash" },
                "proof":    { "type": "string", "description": "Base64 STARK proof bytes" }
            },
            "required": ["intentId", "proof"]
        }),
        Some(McpTransport::StreamableHttp),
        Some(json!({
            "intentId": "0xfeedface...",
            "proof": "base64..."
        })),
        None,
    );

    println!(
        "[demo] built MCP bazaar extension:\n{}",
        serde_json::to_string_pretty(&mcp_ext)?
    );

    let http = reqwest::Client::new();
    let catalog_resp = http
        .post(format!("{}/catalog", base))
        .json(&json!({
            "resource_url": "https://verifier.vesl.cloud/mcp/verify_intent_settlement",
            "extension": mcp_ext,
        }))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    println!("[demo] catalog response: {}", catalog_resp);

    let client = BazaarClient::new(base);
    let result = client
        .list_resources(ListDiscoveryResourcesParams {
            kind: Some("mcp".into()),
            limit: Some(10),
            offset: Some(0),
        })
        .await?;

    println!(
        "[demo] discovery query returned {} item(s):\n{}",
        result.items.len(),
        serde_json::to_string_pretty(&result)?
    );

    assert_eq!(result.items.len(), 1, "expected 1 catalogued resource");
    assert_eq!(result.pagination.total, 1);
    assert_eq!(result.items[0].kind, "mcp");
    assert!(result.items[0]
        .resource
        .ends_with("/verify_intent_settlement"));

    println!("[demo] OK — round trip succeeded");
    Ok(())
}
