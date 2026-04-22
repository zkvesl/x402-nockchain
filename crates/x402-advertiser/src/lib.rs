//! Server-side helpers for x402's Bazaar discovery extension.
//!
//! Provides builders that produce [`BazaarExtension`] blocks for attachment
//! to 402 responses. Mirrors upstream `declareDiscoveryExtension` semantics
//! (one builder, three input flavours: HTTP query, HTTP body, MCP tool).
//!
//! Reference: `coinbase/x402:specs/extensions/bazaar.md`.

use serde_json::Value;
use std::collections::BTreeMap;
use x402_types::{
    BazaarExtension, BodyMethod, BodyType, DiscoveryInfo, DiscoveryInput, DiscoveryOutput,
    HttpInput, McpInput, McpTransport, QueryMethod,
};

/// Build a `bazaar` extension for an HTTP query-param endpoint
/// (`GET` / `HEAD` / `DELETE`).
pub fn declare_http_query(
    method: QueryMethod,
    query_params: Option<BTreeMap<String, Value>>,
    headers: Option<BTreeMap<String, String>>,
    output: Option<DiscoveryOutput>,
) -> BazaarExtension {
    let info = DiscoveryInfo {
        input: DiscoveryInput::Http(HttpInput::Query {
            method,
            query_params: query_params.clone(),
            headers: headers.clone(),
        }),
        output: output.clone(),
    };
    BazaarExtension {
        schema: build_query_schema(method, &query_params, &headers, &output),
        info,
    }
}

/// Build a `bazaar` extension for an HTTP body endpoint
/// (`POST` / `PUT` / `PATCH`).
pub fn declare_http_body(
    method: BodyMethod,
    body_type: BodyType,
    body: Value,
    query_params: Option<BTreeMap<String, Value>>,
    headers: Option<BTreeMap<String, String>>,
    output: Option<DiscoveryOutput>,
) -> BazaarExtension {
    let info = DiscoveryInfo {
        input: DiscoveryInput::Http(HttpInput::Body {
            method,
            body_type,
            body: body.clone(),
            query_params: query_params.clone(),
            headers: headers.clone(),
        }),
        output: output.clone(),
    };
    BazaarExtension {
        schema: build_body_schema(method, body_type, &body, &query_params, &headers, &output),
        info,
    }
}

/// Build a `bazaar` extension for an MCP tool.
pub fn declare_mcp(
    tool: impl Into<String>,
    description: Option<String>,
    input_schema: Value,
    transport: Option<McpTransport>,
    example: Option<Value>,
    output: Option<DiscoveryOutput>,
) -> BazaarExtension {
    let info = DiscoveryInfo {
        input: DiscoveryInput::Mcp(McpInput {
            tool: tool.into(),
            description: description.clone(),
            input_schema: input_schema.clone(),
            transport,
            example: example.clone(),
        }),
        output: output.clone(),
    };
    BazaarExtension {
        schema: build_mcp_schema(&input_schema, transport, &output),
        info,
    }
}

fn build_query_schema(
    _method: QueryMethod,
    query_params: &Option<BTreeMap<String, Value>>,
    _headers: &Option<BTreeMap<String, String>>,
    output: &Option<DiscoveryOutput>,
) -> Value {
    let qp_props = query_params.as_ref().map(|qp| {
        let props: serde_json::Map<String, Value> = qp
            .keys()
            .map(|k| (k.clone(), serde_json::json!({ "type": "string" })))
            .collect();
        Value::Object(props)
    });
    let mut input = serde_json::json!({
        "type": "object",
        "properties": {
            "type": { "type": "string", "const": "http" },
            "method": { "type": "string", "enum": ["GET", "HEAD", "DELETE"] },
        },
        "required": ["type", "method"],
        "additionalProperties": false,
    });
    if let Some(qp) = qp_props {
        input["properties"]["queryParams"] = serde_json::json!({
            "type": "object",
            "properties": qp,
        });
    }
    let mut schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "input": input },
        "required": ["input"],
    });
    if output.is_some() {
        schema["properties"]["output"] = serde_json::json!({
            "type": "object",
            "properties": { "type": { "type": "string" } },
            "required": ["type"],
        });
    }
    schema
}

fn build_body_schema(
    _method: BodyMethod,
    _body_type: BodyType,
    _body: &Value,
    _query_params: &Option<BTreeMap<String, Value>>,
    _headers: &Option<BTreeMap<String, String>>,
    output: &Option<DiscoveryOutput>,
) -> Value {
    let mut schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "input": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "const": "http" },
                    "method": { "type": "string", "enum": ["POST", "PUT", "PATCH"] },
                    "bodyType": { "type": "string", "enum": ["json", "form-data", "text"] },
                    "body": {},
                },
                "required": ["type", "method", "bodyType", "body"],
            },
        },
        "required": ["input"],
    });
    if output.is_some() {
        schema["properties"]["output"] = serde_json::json!({
            "type": "object",
            "properties": { "type": { "type": "string" } },
            "required": ["type"],
        });
    }
    schema
}

fn build_mcp_schema(
    _input_schema: &Value,
    _transport: Option<McpTransport>,
    output: &Option<DiscoveryOutput>,
) -> Value {
    let mut schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "input": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "const": "mcp" },
                    "tool": { "type": "string" },
                    "description": { "type": "string" },
                    "transport": { "type": "string", "enum": ["streamable-http", "sse"] },
                    "inputSchema": { "type": "object" },
                    "example": { "type": "object" },
                },
                "required": ["type", "tool", "inputSchema"],
                "additionalProperties": false,
            },
        },
        "required": ["input"],
    });
    if output.is_some() {
        schema["properties"]["output"] = serde_json::json!({
            "type": "object",
            "properties": { "type": { "type": "string" } },
            "required": ["type"],
        });
    }
    schema
}
