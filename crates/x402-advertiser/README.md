# x402-advertiser

Server-side helpers for x402's Bazaar discovery extension. Three builders that produce a `bazaar` extension block ready to attach to a 402 response: `declare_http_query`, `declare_http_body`, `declare_mcp`.

Mirrors upstream `declareDiscoveryExtension` semantics from `coinbase/x402:typescript/.../bazaar/`. Network-neutral; pairs with [`x402-nockchain-facilitator`](../x402-nockchain-facilitator) on the verifier side.

## Usage

Build a bazaar extension for an MCP echo tool, attach it to a 402 response:

```rust
use serde_json::json;
use x402_advertiser::declare_mcp;
use x402_types::McpTransport;

let bazaar = declare_mcp(
    "echo",
    Some("Echo input back to the caller".into()),
    json!({
        "type": "object",
        "properties": { "message": { "type": "string" } },
        "required": ["message"]
    }),
    Some(McpTransport::StreamableHttp),
    Some(json!({ "message": "hi" })),
    None,
);

// `bazaar` is a `BazaarExtension { info, schema }` ready to be attached
// to the `extensions` map of a `PaymentRequired` 402 envelope.
assert!(bazaar.schema.is_object());
```

For HTTP endpoints, use `declare_http_query` (GET / HEAD / DELETE) or `declare_http_body` (POST / PUT / PATCH). Each builder hand-rolls the JSON Schema describing its `info` block — the facilitator validates incoming `info` against this schema during the bazaar extension's catalog step.

The hand-rolled schema choice is documented in [ADR-0014](../../docs/decisions/0014-handrolled-jsonschema-deferred-schemars.md); a `schemars` evaluation is queued for post-Phase-5.

## Wiring

`x402-advertiser` produces the extension block; attaching it to a 402 response is the caller's responsibility. The `examples/e2e_demo` binary and the `x402-mcp` crate both show working integrations.
