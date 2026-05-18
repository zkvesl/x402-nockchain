# x402-mcp

MCP (Model Context Protocol) adapter for x402: wraps a tool registry and emits 402 responses with bazaar extension blocks for unpaid tool calls.

Three layers:

| Layer | Role |
|---|---|
| `registry` | `McpToolRegistry` holds tool definitions (name, description, `inputSchema`, handler) plus the `PaymentRequirements` and pre-built `BazaarExtension` blocks. Pure: no network, no verification. |
| `verifier` | `PaymentVerifier` trait — pluggable. Two impls ship: `AlwaysAcceptVerifier` (tests) and `RemoteFacilitatorVerifier` (POSTs to a configured facilitator `/verify`). |
| `router` | `router_for_registry` wires both into an `axum::Router` exposing `POST /mcp/call/:tool`. |

## Wire shape

`POST /mcp/call/:tool` — pragmatic MCP-over-HTTP binding. Unsigned or signature-failing requests get `402 Payment Required` with the tool's bazaar extension echoed in. Signed, verified requests proxy to the tool handler. MCP's native JSON-RPC-over-SSE transport is a future extension; the registry's `Tool.inputSchema` shape is canonical, so plugging in SSE later is a transport change rather than a registry change.

## Usage

Build a registry, register one tool, wire a router:

```rust no_run
use std::sync::Arc;
use serde_json::json;
use x402_mcp::{
    router_for_registry, AlwaysAcceptVerifier, McpToolRegistry, PaymentVerifier,
    ToolHandler,
};
use x402_types::payment::{PaymentRequirements, PaymentResource};
use x402_types::McpTransport;

let resource = PaymentResource {
    url: "https://api.example.com/mcp/call/echo".into(),
    description: Some("MCP echo".into()),
    mime_type: Some("application/json".into()),
};
let accepts = vec![PaymentRequirements {
    scheme: "exact".into(),
    network: "nockchain:mainnet".into(),
    max_amount_required: "65536".into(),
    resource: resource.url.clone(),
    asset: "NOCK".into(),
    pay_to: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".into(),
    max_timeout_seconds: 60,
    description: Some("Echo".into()),
    mime_type: Some("application/json".into()),
    output_schema: None,
    extra: None,
    extensions: None,
}];

let mut registry = McpToolRegistry::new(resource, accepts);
let handler: ToolHandler = Arc::new(|args| {
    Box::pin(async move { Ok(json!({ "echoed": args })) })
});
registry
    .register(
        "echo",
        Some("Echo a string".into()),
        json!({
            "type": "object",
            "properties": { "message": { "type": "string" } },
            "required": ["message"]
        }),
        Some(McpTransport::StreamableHttp),
        handler,
    )
    .expect("register");

let verifier: Arc<dyn PaymentVerifier> = Arc::new(AlwaysAcceptVerifier);
let _router = router_for_registry(Arc::new(registry), verifier);
// `axum::serve(listener, router)` from here.
```

For a full end-to-end example with a real facilitator + real signer, see `examples/e2e_demo/src/bin/demo_full.rs`.
