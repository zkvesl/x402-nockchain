//! Roundtrip every JSON example from the pinned spec snapshots
//! (`docs/specs-snapshot/`) through the typed structs in this crate.
//!
//! Each test:
//!   1. Parses the fixture JSON into `serde_json::Value`.
//!   2. Deserializes it into the corresponding typed struct.
//!   3. Re-serializes the typed struct back to `Value`.
//!   4. Asserts the before/after are byte-identical after canonicalization
//!      (serialized via `serde_json`, whose `Map` is a `BTreeMap` —
//!      object keys come out lexicographically sorted).
//!
//! Examples that are fragments (spec-illustrative but not valid standalone
//! typed structs — e.g. `"accepts": [ ... ]` placeholders) are extracted to
//! the sub-object they do type-model and tested in that form.

use pretty_assertions::assert_eq;
use serde_json::Value;
use x402_types::bazaar::BazaarExtension;
use x402_types::payment::{
    ExtensionResponsesHeader, PaymentRequired, PaymentRequirements, SchnorrSignatureJson,
};
use x402_types::siwn::SiwnExtra;

#[cfg(feature = "nockchain")]
use x402_types::nockchain::ExactNockchainPayload;
#[cfg(feature = "nockchain")]
use x402_types::payment::PaymentPayload;

fn canonicalize(v: &Value) -> String {
    serde_json::to_string(v).expect("serialize for canonical form")
}

macro_rules! roundtrip_case {
    ($name:ident, $ty:ty, $json:expr) => {
        #[test]
        fn $name() {
            let src: Value = serde_json::from_str($json).expect("parse fixture JSON");
            let typed: $ty = serde_json::from_value(src.clone()).expect(concat!(
                "deserialize fixture into ",
                stringify!($ty)
            ));
            let back = serde_json::to_value(&typed).expect("re-serialize typed form");
            assert_eq!(canonicalize(&src), canonicalize(&back));
        }
    };
}

// ---------------------------------------------------------------------------
// 05-payload.md §5.8 — Full PaymentPayload with Nockchain exact payload
// ---------------------------------------------------------------------------
#[cfg(feature = "nockchain")]
roundtrip_case!(
    payment_payload_full_example_5_8,
    PaymentPayload<ExactNockchainPayload>,
    r#"{
  "x402Version": 2,
  "scheme": "exact",
  "network": "nockchain:mainnet",
  "payload": {
    "signature": {
      "pubkey": "5Ht7Rk3qX9...",
      "schnorr": {
        "chal": ["12345678901234567", "12345678901234567", "12345678901234567", "12345678901234567",
                  "12345678901234567", "12345678901234567", "12345678901234567", "12345678901234567"],
        "sig":  ["12345678901234567", "12345678901234567", "12345678901234567", "12345678901234567",
                  "12345678901234567", "12345678901234567", "12345678901234567", "12345678901234567"]
      }
    },
    "authorization": {
      "from": "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
      "to": "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
      "value": "65536",
      "fee": "10",
      "nonce": "2NEpo7TZRhna7JNR...",
      "validAfter": 1708000000,
      "validBefore": 1708000300,
      "notes": [
        {
          "name": {
            "first": "4vJ9JU1bJJE...",
            "last": "7iYDhLfEgN..."
          },
          "assets": "131072"
        }
      ],
      "changeAddress": "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
    }
  }
}"#
);

// ---------------------------------------------------------------------------
// 11-extensions.md §11.2.3 — inner `siwn` block of `PaymentRequirements.extra`
// ---------------------------------------------------------------------------
roundtrip_case!(
    siwn_extra_block_11_2_3,
    SiwnExtra,
    r#"{
        "domain": "api.example.com",
        "nonce": "<random-nonce>",
        "issuedAt": "2026-02-17T12:00:00Z",
        "expirationTime": "2026-02-17T13:00:00Z"
    }"#
);

// ---------------------------------------------------------------------------
// bazaar.md — EXTENSION-RESPONSES header examples (decoded from base64)
// ---------------------------------------------------------------------------
roundtrip_case!(
    extension_responses_header_success,
    ExtensionResponsesHeader,
    r#"{"bazaar":{"status":"success"}}"#
);

roundtrip_case!(
    extension_responses_header_rejected,
    ExtensionResponsesHeader,
    r#"{"bazaar":{"status":"rejected","rejectedReason":"info failed schema validation"}}"#
);

// ---------------------------------------------------------------------------
// bazaar.md — `extensions.bazaar` object extracted from each full example
// ---------------------------------------------------------------------------
roundtrip_case!(
    bazaar_extension_http_get,
    BazaarExtension,
    r#"{
        "info": {
            "input": {
                "type": "http",
                "method": "GET",
                "queryParams": {
                    "city": "San Francisco"
                }
            },
            "output": {
                "type": "json",
                "example": {
                    "city": "San Francisco",
                    "weather": "foggy",
                    "temperature": 60
                }
            }
        },
        "schema": {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "input": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "const": "http" },
                        "method": { "type": "string", "enum": ["GET", "HEAD", "DELETE"] },
                        "queryParams": {
                            "type": "object",
                            "properties": {
                                "city": { "type": "string" }
                            },
                            "required": ["city"]
                        },
                        "headers": {
                            "type": "object",
                            "additionalProperties": { "type": "string" }
                        }
                    },
                    "required": ["type", "method"],
                    "additionalProperties": false
                },
                "output": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string" },
                        "example": { "type": "object" }
                    },
                    "required": ["type"]
                }
            },
            "required": ["input"]
        }
    }"#
);

roundtrip_case!(
    bazaar_extension_http_post,
    BazaarExtension,
    r#"{
        "info": {
            "input": {
                "type": "http",
                "method": "POST",
                "bodyType": "json",
                "body": { "query": "example" }
            },
            "output": {
                "type": "json",
                "example": { "results": [] }
            }
        },
        "schema": {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "input": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "const": "http" },
                        "method": { "type": "string", "enum": ["POST", "PUT", "PATCH"] },
                        "bodyType": { "type": "string", "enum": ["json", "form-data", "text"] },
                        "body": { "type": "object" }
                    },
                    "required": ["type", "method", "bodyType", "body"]
                }
            },
            "required": ["input"]
        }
    }"#
);

roundtrip_case!(
    bazaar_extension_mcp,
    BazaarExtension,
    r#"{
        "info": {
            "input": {
                "type": "mcp",
                "tool": "financial_analysis",
                "description": "Advanced AI-powered financial analysis",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ticker": { "type": "string" },
                        "analysis_type": { "type": "string", "enum": ["quick", "deep"] }
                    },
                    "required": ["ticker"]
                },
                "example": { "ticker": "AAPL", "analysis_type": "deep" }
            },
            "output": {
                "type": "json",
                "example": { "summary": "Strong fundamentals...", "score": 8.5 }
            }
        },
        "schema": {
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
                        "example": { "type": "object" }
                    },
                    "required": ["type", "tool", "inputSchema"],
                    "additionalProperties": false
                }
            },
            "required": ["input"]
        }
    }"#
);

// ---------------------------------------------------------------------------
// Regression — camelCase wire format for snake_case Rust fields
// (phase-1 risk note "Field name casing")
// ---------------------------------------------------------------------------
#[test]
fn payment_requirements_wire_format_is_camelcase() {
    let req = PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:mainnet".into(),
        max_amount_required: "65536".into(),
        resource: "https://api.example.com/weather".into(),
        asset: "NOCK".into(),
        pay_to: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".into(),
        max_timeout_seconds: 60,
        description: Some("Weather data".into()),
        mime_type: Some("application/json".into()),
        output_schema: None,
        extra: None,
        extensions: None,
    };
    let s = serde_json::to_string(&req).expect("serialize PaymentRequirements");

    // Renamed snake_case → camelCase fields must be present in camelCase.
    assert!(s.contains("\"maxAmountRequired\""), "missing maxAmountRequired: {s}");
    assert!(s.contains("\"payTo\""), "missing payTo: {s}");
    assert!(s.contains("\"maxTimeoutSeconds\""), "missing maxTimeoutSeconds: {s}");
    assert!(s.contains("\"mimeType\""), "missing mimeType: {s}");

    // Rust snake_case spellings must NOT leak onto the wire.
    assert!(!s.contains("max_amount_required"), "snake_case leaked: {s}");
    assert!(!s.contains("pay_to"), "snake_case leaked: {s}");
    assert!(!s.contains("max_timeout_seconds"), "snake_case leaked: {s}");
    assert!(!s.contains("mime_type"), "snake_case leaked: {s}");

    // Non-renamed fields (identifier already matches wire) are present as-is.
    assert!(s.contains("\"scheme\""));
    assert!(s.contains("\"network\""));
    assert!(s.contains("\"asset\""));
}

// ---------------------------------------------------------------------------
// bazaar.md — PaymentRequired 402 envelope (accepts placeholder substituted
// with a concrete PaymentRequirements object)
// ---------------------------------------------------------------------------
roundtrip_case!(
    payment_required_envelope,
    PaymentRequired,
    r#"{
        "x402Version": 2,
        "error": "Payment required",
        "resource": {
            "url": "https://api.example.com/weather",
            "description": "Weather data endpoint",
            "mimeType": "application/json"
        },
        "accepts": [
            {
                "scheme": "exact",
                "network": "nockchain:mainnet",
                "maxAmountRequired": "65536",
                "resource": "https://api.example.com/weather",
                "asset": "NOCK",
                "payTo": "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
                "maxTimeoutSeconds": 60
            }
        ],
        "extensions": {
            "bazaar": {
                "info": {
                    "input": {
                        "type": "http",
                        "method": "GET",
                        "queryParams": { "city": "San Francisco" }
                    }
                },
                "schema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object"
                }
            }
        }
    }"#
);

// ---------------------------------------------------------------------------
// SchnorrSignatureJson helpers
// ---------------------------------------------------------------------------
#[test]
fn schnorr_signature_all_zero_is_all_zero() {
    let sig = SchnorrSignatureJson::all_zero("stubpubkey");
    assert!(sig.is_all_zero());
    assert_eq!(sig.pubkey, "stubpubkey");
    for s in sig.schnorr.chal.iter().chain(sig.schnorr.sig.iter()) {
        assert_eq!(s, "0");
    }
}

#[test]
fn schnorr_signature_is_all_zero_rejects_nonzero() {
    let mut sig = SchnorrSignatureJson::all_zero("x");
    sig.schnorr.sig[3] = "42".into();
    assert!(!sig.is_all_zero());
}

#[test]
fn authorization_wire_format_is_camelcase() {
    let v: Value = serde_json::from_str(
        r#"{
            "from": "a",
            "to": "b",
            "value": "1",
            "fee": "1",
            "nonce": "n",
            "validAfter": 1,
            "validBefore": 2,
            "notes": [{"name":{"first":"f","last":"l"},"assets":"1"}],
            "changeAddress": "c"
        }"#,
    )
    .unwrap();
    let typed: x402_types::payment::Authorization = serde_json::from_value(v).unwrap();
    let s = serde_json::to_string(&typed).unwrap();

    assert!(s.contains("\"validAfter\""));
    assert!(s.contains("\"validBefore\""));
    assert!(s.contains("\"changeAddress\""));
    assert!(!s.contains("valid_after"));
    assert!(!s.contains("valid_before"));
    assert!(!s.contains("change_address"));
}
