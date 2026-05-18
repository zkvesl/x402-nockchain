//! axum router that fronts an [`McpToolRegistry`].
//!
//! - `POST /mcp/call/:tool` — tool invocation. Reads the
//!   `X-PAYMENT-SIGNATURE` header (base64(JSON `PaymentPayload`)). If
//!   absent or rejected by the verifier, returns `402 Payment Required`
//!   with the registry's `PaymentRequired` body (echoing the tool's
//!   bazaar extension). If accepted, the tool handler runs and returns
//!   200 / 500.
//! - `GET /mcp/tools` — discovery helper: returns the list of registered
//!   tools plus their input schemas and bazaar blocks. Not
//!   payment-gated — it's the same information a client would see in a
//!   402 response, surfaced proactively for agents.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::Value;
use x402_types::payment::PaymentPayload;

use crate::registry::{CallError, CallOutcome, McpToolRegistry};
use crate::verifier::PaymentVerifier;

pub const PAYMENT_SIGNATURE_HEADER: &str = "x-payment-signature";

/// Shared state for the MCP router.
#[derive(Clone)]
pub struct McpRouterState {
    pub registry: Arc<McpToolRegistry>,
    pub verifier: Arc<dyn PaymentVerifier>,
}

pub fn router_for_registry(
    registry: Arc<McpToolRegistry>,
    verifier: Arc<dyn PaymentVerifier>,
) -> Router {
    let state = McpRouterState {
        registry,
        verifier,
    };
    Router::new()
        .route("/mcp/call/:tool", post(call_tool))
        .route("/mcp/tools", get(list_tools))
        .with_state(state)
}

async fn list_tools(State(state): State<McpRouterState>) -> Json<Value> {
    let tools: Vec<_> = state
        .registry
        .tools()
        .map(|t| {
            serde_json::json!({
                "name": t.tool,
                "description": t.description,
                "inputSchema": t.input_schema,
                "transport": t.transport,
                "bazaar": t.bazaar,
            })
        })
        .collect();
    Json(serde_json::json!({ "tools": tools }))
}

async fn call_tool(
    State(state): State<McpRouterState>,
    Path(tool): Path<String>,
    headers: HeaderMap,
    Json(args): Json<Value>,
) -> axum::response::Response {
    let registered = match state.registry.get(&tool) {
        Some(r) => r,
        None => return not_found(&tool),
    };

    let requirements = match state.registry.accepts().first() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "registry has no PaymentRequirements configured",
                })),
            )
                .into_response();
        }
    };

    let payment_header = headers
        .get(PAYMENT_SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let payload = match payment_header {
        None => return payment_required(&state.registry, &tool),
        Some(h) => match decode_payment_header(&h) {
            Ok(p) => p,
            Err(_) => return payment_required(&state.registry, &tool),
        },
    };

    if let Err(_rej) = state.verifier.verify(&payload, requirements).await {
        return payment_required(&state.registry, &tool);
    }

    match state.registry.invoke(&registered.tool, args).await {
        Ok(CallOutcome::Success(v)) => Json(v).into_response(),
        Ok(CallOutcome::ToolFailed(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.message })),
        )
            .into_response(),
        Err(CallError::UnknownTool(t)) => not_found(&t),
        Err(CallError::Tool(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.message })),
        )
            .into_response(),
    }
}

fn decode_payment_header(h: &str) -> Result<PaymentPayload<Value>, anyhow::Error> {
    let bytes = B64
        .decode(h)
        .map_err(|e| anyhow::anyhow!("base64 decode: {e}"))?;
    let payload: PaymentPayload<Value> = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("payment payload decode: {e}"))?;
    Ok(payload)
}

fn payment_required(registry: &McpToolRegistry, tool: &str) -> axum::response::Response {
    match registry.payment_required(tool) {
        Some(body) => (StatusCode::PAYMENT_REQUIRED, Json(body)).into_response(),
        None => not_found(tool),
    }
}

fn not_found(tool: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("unknown tool '{tool}'") })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolHandler;
    use crate::verifier::AlwaysAcceptVerifier;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    use x402_types::payment::{PaymentRequirements, PaymentResource};
    use x402_types::McpTransport;

    fn mk_state() -> McpRouterState {
        let resource = PaymentResource {
            url: "https://example/mcp".into(),
            description: None,
            mime_type: None,
        };
        let accepts = vec![PaymentRequirements {
            scheme: "exact".into(),
            network: "nockchain:fakenet".into(),
            max_amount_required: "1".into(),
            resource: "https://example/mcp".into(),
            asset: "NOCK".into(),
            pay_to: "2kPay".into(),
            max_timeout_seconds: 30,
            description: None,
            mime_type: None,
            output_schema: None,
            extra: None,
            extensions: None,
        }];
        let mut reg = McpToolRegistry::new(resource, accepts);
        let handler: ToolHandler = Arc::new(|args: Value| {
            Box::pin(async move { Ok(args) })
        });
        reg.register(
            "echo",
            None,
            serde_json::json!({ "type": "object" }),
            Some(McpTransport::StreamableHttp),
            handler,
        )
        .unwrap();
        McpRouterState {
            registry: Arc::new(reg),
            verifier: Arc::new(AlwaysAcceptVerifier),
        }
    }

    #[tokio::test]
    async fn unpaid_call_returns_402_with_bazaar() {
        let state = mk_state();
        let router = router_for_registry(state.registry.clone(), state.verifier.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/call/echo")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"msg":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["x402Version"], 2);
        assert!(body["error"].as_str().unwrap().contains("echo"));
        assert!(body["extensions"]["bazaar"].is_object());
    }

    #[tokio::test]
    async fn paid_call_invokes_handler() {
        let state = mk_state();
        let router = router_for_registry(state.registry.clone(), state.verifier.clone());

        let payload = PaymentPayload {
            x402_version: 2,
            scheme: "exact".into(),
            network: "nockchain:fakenet".into(),
            payload: Value::Null,
            extensions: None,
        };
        let encoded = B64.encode(serde_json::to_vec(&payload).unwrap());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/call/echo")
                    .header("content-type", "application/json")
                    .header(PAYMENT_SIGNATURE_HEADER, encoded)
                    .body(Body::from(r#"{"msg":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body, serde_json::json!({"msg": "hi"}));
    }

    #[tokio::test]
    async fn unknown_tool_returns_404() {
        let state = mk_state();
        let router = router_for_registry(state.registry.clone(), state.verifier.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/call/missing")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_tools_returns_registered_entries() {
        let state = mk_state();
        let router = router_for_registry(state.registry.clone(), state.verifier.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/mcp/tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body_bytes).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo");
    }
}
