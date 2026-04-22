//! MCP adapter for x402.
//!
//! Scaffold at M0; implementation begins at M4 per
//! `vesl-agent/docs/plans/phase-4-testnet-mcp.md`. Will wrap an MCP tool
//! registry and emit `402 Payment Required` responses with Bazaar extension
//! blocks for unpaid tool calls.
