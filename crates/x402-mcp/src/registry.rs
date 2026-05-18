//! Tool registry + 402 builder. Network- and verifier-free by design.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use x402_advertiser::declare_mcp;
use x402_types::bazaar::BazaarExtension;
use x402_types::payment::{PaymentRequired, PaymentRequirements, PaymentResource};
use x402_types::McpTransport;

/// Boxed future returned by a tool handler. Static lifetime so it composes
/// cleanly in an axum handler.
pub type BoxToolFuture = Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send>>;

/// Handler closure signature. Takes the JSON args (validated against the
/// tool's `inputSchema` by the router), returns the tool result or a
/// `ToolError`.
pub type ToolHandler = Arc<dyn Fn(Value) -> BoxToolFuture + Send + Sync>;

/// An entry in the [`McpToolRegistry`].
#[derive(Clone)]
pub struct RegisteredTool {
    pub tool: String,
    pub description: Option<String>,
    pub input_schema: Value,
    pub transport: Option<McpTransport>,
    pub bazaar: BazaarExtension,
    pub handler: ToolHandler,
}

impl std::fmt::Debug for RegisteredTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredTool")
            .field("tool", &self.tool)
            .field("description", &self.description)
            .field("input_schema", &self.input_schema)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

/// Registry of MCP tools behind a common set of [`PaymentRequirements`].
///
/// Each registered tool carries a pre-built `BazaarExtension` so the 402
/// responses we mint name the specific tool, not the whole registry.
pub struct McpToolRegistry {
    resource: PaymentResource,
    accepts: Vec<PaymentRequirements>,
    tools: BTreeMap<String, RegisteredTool>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("tool '{0}' already registered")]
    Duplicate(String),
}

/// Errors raised by a tool handler itself. Propagates to the caller as a
/// 500-class response.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    pub message: String,
}

impl ToolError {
    pub fn new(m: impl Into<String>) -> Self {
        Self { message: m.into() }
    }
}

/// Errors from calling a tool through the registry.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool handler failed: {0}")]
    Tool(#[from] ToolError),
}

/// Outcome of [`McpToolRegistry::invoke`]. The router decides the HTTP
/// shape — 200 for `Success`, 500 for `ToolFailed`.
#[derive(Debug)]
pub enum CallOutcome {
    Success(Value),
    ToolFailed(ToolError),
}

impl McpToolRegistry {
    /// Create an empty registry bound to a resource URL + a non-empty list
    /// of `accepts` entries (the `402.accepts` array). The `accepts` list
    /// is what each 402 response echoes back verbatim.
    pub fn new(
        resource: PaymentResource,
        accepts: Vec<PaymentRequirements>,
    ) -> Self {
        Self {
            resource,
            accepts,
            tools: BTreeMap::new(),
        }
    }

    /// Register a new tool. Returns an error on duplicate name.
    pub fn register(
        &mut self,
        tool: impl Into<String>,
        description: Option<String>,
        input_schema: Value,
        transport: Option<McpTransport>,
        handler: ToolHandler,
    ) -> Result<(), RegisterError> {
        let name: String = tool.into();
        if self.tools.contains_key(&name) {
            return Err(RegisterError::Duplicate(name));
        }
        let bazaar = declare_mcp(
            name.clone(),
            description.clone(),
            input_schema.clone(),
            transport,
            None,
            None,
        );
        self.tools.insert(
            name.clone(),
            RegisteredTool {
                tool: name,
                description,
                input_schema,
                transport,
                bazaar,
                handler,
            },
        );
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    pub fn tools(&self) -> impl Iterator<Item = &RegisteredTool> {
        self.tools.values()
    }

    pub fn accepts(&self) -> &[PaymentRequirements] {
        &self.accepts
    }

    pub fn resource(&self) -> &PaymentResource {
        &self.resource
    }

    /// Build the `402 Payment Required` body for a given tool. Fails if
    /// the tool isn't registered.
    pub fn payment_required(&self, tool: &str) -> Option<PaymentRequired> {
        let registered = self.tools.get(tool)?;
        let mut extensions = BTreeMap::new();
        extensions.insert(
            "bazaar".to_string(),
            serde_json::to_value(&registered.bazaar).ok()?,
        );
        Some(PaymentRequired {
            x402_version: 2,
            error: format!("Payment required for tool '{tool}'"),
            resource: self.resource.clone(),
            accepts: self.accepts.clone(),
            extensions: Some(extensions),
        })
    }

    /// Invoke a registered tool with the given args. No payment check —
    /// the router layer owns that decision.
    pub async fn invoke(&self, tool: &str, args: Value) -> Result<CallOutcome, CallError> {
        let registered = self
            .tools
            .get(tool)
            .ok_or_else(|| CallError::UnknownTool(tool.to_string()))?;
        match (registered.handler)(args).await {
            Ok(v) => Ok(CallOutcome::Success(v)),
            Err(e) => Ok(CallOutcome::ToolFailed(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_resource() -> PaymentResource {
        PaymentResource {
            url: "https://example/mcp".into(),
            description: Some("Example server".into()),
            mime_type: None,
        }
    }

    fn fixture_requirements() -> Vec<PaymentRequirements> {
        vec![PaymentRequirements {
            scheme: "exact".into(),
            network: "nockchain:fakenet".into(),
            max_amount_required: "1000".into(),
            resource: "https://example/mcp".into(),
            asset: "NOCK".into(),
            pay_to: "2kEcho".into(),
            max_timeout_seconds: 30,
            description: None,
            mime_type: None,
            output_schema: None,
            extra: None,
            extensions: None,
        }]
    }

    fn make_registry() -> McpToolRegistry {
        let mut reg = McpToolRegistry::new(fixture_resource(), fixture_requirements());
        reg.register(
            "echo",
            Some("Echo a message".into()),
            json!({ "type": "object", "properties": { "msg": { "type": "string" } }, "required": ["msg"] }),
            Some(McpTransport::StreamableHttp),
            Arc::new(|args: Value| {
                Box::pin(async move { Ok(args) })
            }),
        )
        .unwrap();
        reg
    }

    #[tokio::test]
    async fn invoke_echos() {
        let reg = make_registry();
        let out = reg.invoke("echo", json!({"msg": "hi"})).await.unwrap();
        match out {
            CallOutcome::Success(v) => assert_eq!(v, json!({"msg": "hi"})),
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let reg = make_registry();
        let err = reg.invoke("nope", json!({})).await.unwrap_err();
        matches!(err, CallError::UnknownTool(_));
    }

    #[tokio::test]
    async fn payment_required_names_the_tool() {
        let reg = make_registry();
        let pr = reg.payment_required("echo").unwrap();
        assert!(pr.error.contains("echo"));
        let ext = pr.extensions.unwrap();
        assert!(ext.contains_key("bazaar"));
    }

    #[test]
    fn duplicate_register_rejected() {
        let mut reg = make_registry();
        let err = reg
            .register(
                "echo",
                None,
                json!({}),
                None,
                Arc::new(|_| Box::pin(async { Ok(Value::Null) })),
            )
            .unwrap_err();
        matches!(err, RegisterError::Duplicate(_));
    }
}
