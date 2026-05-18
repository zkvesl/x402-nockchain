//! MCP adapter for x402.
//!
//! Three layers:
//!
//! - [`registry`]: an `McpToolRegistry` holds tool definitions (name,
//!   description, `inputSchema`, handler) plus the `PaymentRequirements`
//!   and pre-built [`x402_types::BazaarExtension`] blocks needed to issue
//!   `402 Payment Required` responses. Pure: no network, no verification.
//! - [`verifier`]: a [`PaymentVerifier`] trait with two shipping impls —
//!   [`AlwaysAcceptVerifier`] for tests and
//!   [`RemoteFacilitatorVerifier`] that POSTs to a configured facilitator
//!   `/verify` endpoint.
//! - [`router`]: [`router_for_registry`] wires both into an `axum::Router`
//!   exposing `POST /mcp/call/:tool`. Unsigned or signature-failing
//!   requests get 402 with the right bazaar extension echoed in; signed,
//!   verified requests are proxied to the tool handler.
//!
//! Phase 4 scope. The wire shape (`POST /mcp/call/:tool`) is a pragmatic
//! MCP-over-HTTP binding — MCP's native JSON-RPC-over-SSE support is a
//! future extension. Tool handlers already use MCP's canonical
//! `Tool.inputSchema` shape, so plugging in an SSE transport later is a
//! transport change, not a registry change.

pub mod registry;
pub mod router;
pub mod verifier;

pub use registry::{
    CallError, CallOutcome, McpToolRegistry, RegisterError, RegisteredTool, ToolError, ToolHandler,
};
pub use router::{router_for_registry, McpRouterState};
pub use verifier::{
    AlwaysAcceptVerifier, PaymentVerifier, RemoteFacilitatorVerifier, VerifyFailure,
};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctest;
