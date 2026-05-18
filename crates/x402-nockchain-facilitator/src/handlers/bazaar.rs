//! Shared bazaar-extension processing used by both `/verify` and `/settle`.
//!
//! When a client echoes the `bazaar` extension from a 402 `PaymentRequired`
//! into its `PaymentPayload`, the facilitator (a) validates `info` against
//! the accompanying JSON Schema Draft 2020-12, (b) upserts the resource
//! into the SQLite catalog, and (c) reports the outcome in the
//! `EXTENSION-RESPONSES` header per
//! `bazaar.md §Verify and Settlement Response Header`.

use axum::http::{HeaderName, HeaderValue};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use x402_types::bazaar::{BazaarExtension, DiscoveryInput, DiscoveryResource};
use x402_types::facilitator::VerifyRequest;
use x402_types::payment::{
    BazaarExtensionResponse, BazaarExtensionStatus, ExtensionResponsesHeader,
};

use crate::observability::{label, metric};
use crate::AppState;

/// Canonical name of the spec header. HTTP is case-insensitive but
/// `axum::http::HeaderName::from_static` requires lowercase.
pub const EXTENSION_RESPONSES_HEADER: HeaderName = HeaderName::from_static("extension-responses");

/// Process the bazaar extension side of a verify/settle request. If the
/// client did not echo a `bazaar` extension, returns `None` — the handler
/// omits the `EXTENSION-RESPONSES` header entirely (spec: MAY).
///
/// Otherwise, validates and catalogs, returning a
/// [`BazaarExtensionResponse`] describing the outcome. The caller turns
/// this into the `EXTENSION-RESPONSES` header value via
/// [`encode_header_value`].
pub async fn process(
    state: &AppState,
    req: &VerifyRequest,
) -> Option<BazaarExtensionResponse> {
    let ext_map = req.payload.extensions.as_ref()?;
    let bazaar_val = ext_map.get("bazaar")?;

    let ext: BazaarExtension = match serde_json::from_value(bazaar_val.clone()) {
        Ok(e) => e,
        Err(e) => return Some(rejected(format!("bazaar extension parse failed: {e}"))),
    };

    if let Err(reason) = validate_info_against_schema(&ext) {
        return Some(rejected(reason));
    }

    let entry = DiscoveryResource {
        resource: req.requirements.resource.clone(),
        kind: discriminate_kind(&ext.info.input).to_string(),
        x402_version: req.payload.x402_version,
        accepts: vec![req.requirements.clone()],
        metadata: serde_json::to_value(&ext.info).ok(),
        last_updated: chrono::Utc::now().to_rfc3339(),
    };

    let kind_label = entry.kind.clone();
    match state.catalog.upsert(&entry).await {
        Ok(()) => {
            metrics::counter!(
                metric::CATALOG_UPSERTS_TOTAL,
                label::OUTCOME => "ok",
                "kind" => kind_label,
            )
            .increment(1);
            Some(BazaarExtensionResponse {
                status: BazaarExtensionStatus::Success,
                rejected_reason: None,
            })
        }
        Err(e) => {
            metrics::counter!(
                metric::CATALOG_UPSERTS_TOTAL,
                label::OUTCOME => "error",
                "kind" => kind_label,
            )
            .increment(1);
            Some(rejected(format!("catalog upsert failed: {e}")))
        }
    }
}

/// Encode a [`BazaarExtensionResponse`] into a `HeaderValue` suitable for
/// the `EXTENSION-RESPONSES` header (base64-encoded JSON per
/// `bazaar.md §Verify and Settlement Response Header`).
pub fn encode_header_value(response: &BazaarExtensionResponse) -> Option<HeaderValue> {
    let envelope = ExtensionResponsesHeader {
        bazaar: Some(response.clone()),
        other: Default::default(),
    };
    let json = serde_json::to_string(&envelope).ok()?;
    let b64 = B64.encode(json);
    HeaderValue::from_str(&b64).ok()
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn rejected(reason: String) -> BazaarExtensionResponse {
    BazaarExtensionResponse {
        status: BazaarExtensionStatus::Rejected,
        rejected_reason: Some(reason),
    }
}

fn discriminate_kind(input: &DiscoveryInput) -> &'static str {
    match input {
        DiscoveryInput::Http(_) => "http",
        DiscoveryInput::Mcp(_) => "mcp",
    }
}

fn validate_info_against_schema(ext: &BazaarExtension) -> Result<(), String> {
    let info_val =
        serde_json::to_value(&ext.info).map_err(|e| format!("info re-serialize failed: {e}"))?;

    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .compile(&ext.schema)
        .map_err(|e| format!("bazaar schema compile failed: {e}"))?;

    if let Err(errors) = compiled.validate(&info_val) {
        let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
        return Err(format!("info failed schema validation: {}", msgs.join("; ")));
    }

    Ok(())
}
