//! Bazaar discovery-extension types.
//!
//! Mirrors the TS/Python/Go reference implementations from
//! `coinbase/x402:specs/extensions/bazaar.md`. At Phase 0, `accepts` is modeled as
//! `Vec<serde_json::Value>` — a known shortcut until Phase 1 replaces it with a
//! typed `Vec<PaymentRequirements>` from [`crate::payment`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Discovery input variants (HTTP query / HTTP body / MCP tool)
// ---------------------------------------------------------------------------

/// HTTP methods that carry parameters in the query string.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum QueryMethod {
    Get,
    Head,
    Delete,
}

/// HTTP methods that carry a request body.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum BodyMethod {
    Post,
    Put,
    Patch,
}

/// Body encoding format for body-method endpoints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BodyType {
    Json,
    FormData,
    Text,
}

/// MCP tool transport. Upstream defaults to `streamable-http` when omitted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    StreamableHttp,
    Sse,
}

/// `info.input` discriminated by `type`. Mirrors the upstream union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DiscoveryInput {
    Http(HttpInput),
    Mcp(McpInput),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HttpInput {
    Query {
        method: QueryMethod,
        #[serde(skip_serializing_if = "Option::is_none", rename = "queryParams")]
        query_params: Option<BTreeMap<String, Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<BTreeMap<String, String>>,
    },
    Body {
        method: BodyMethod,
        #[serde(rename = "bodyType")]
        body_type: BodyType,
        body: Value,
        #[serde(skip_serializing_if = "Option::is_none", rename = "queryParams")]
        query_params: Option<BTreeMap<String, Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<BTreeMap<String, String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpInput {
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<McpTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryOutput {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
}

/// `info` payload of a `bazaar` extension block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryInfo {
    pub input: DiscoveryInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<DiscoveryOutput>,
}

/// Full `bazaar` extension block: `{ info, schema }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BazaarExtension {
    pub info: DiscoveryInfo,
    /// JSON Schema (Draft 2020-12) validating the structure of `info`.
    pub schema: Value,
}

// ---------------------------------------------------------------------------
// Discovery query API — mirrors upstream `facilitatorClient.ts`
// ---------------------------------------------------------------------------

/// Query parameters for `GET /discovery/resources`. All optional.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListDiscoveryResourcesParams {
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

/// A single catalog entry returned by `/discovery/resources`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryResource {
    pub resource: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "x402Version")]
    pub x402_version: u32,
    /// `PaymentRequirements[]` — `Vec<Value>` at Phase 0, upgraded to
    /// `Vec<PaymentRequirements>` at Phase 1 alongside [`crate::payment`].
    pub accepts: Vec<Value>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub limit: u32,
    pub offset: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryResourcesResponse {
    #[serde(rename = "x402Version")]
    pub x402_version: u32,
    pub items: Vec<DiscoveryResource>,
    pub pagination: Pagination,
}
