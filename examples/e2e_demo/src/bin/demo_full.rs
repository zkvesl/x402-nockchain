//! Phase-4 end-to-end demo: MCP tool priced with x402, settled through a
//! real (or stub) Nockchain chain client.
//!
//! Flow:
//!   1. Boot the facilitator with the chosen ChainClient.
//!   2. Boot an x402-mcp-backed MCP server exposing an `echo` tool.
//!   3. Client A: POST unpaid → expect 402 with bazaar extension.
//!   4. Client A: sign + retry → expect 200 with tool result.
//!   5. Server: POST facilitator `/settle` → log chain status.
//!   6. Client B: GET `/discovery/resources?type=mcp` → expect `echo` listed.
//!
//! Modes:
//!   --mode stub (default): ChainClient is [`StubChainClient`].
//!     Fakenet not required. `/settle` returns `broadcast` with a
//!     deterministic tx_id.
//!   --mode grpc: ChainClient connects to a real Nockchain node.
//!     Requires `--grpc-endpoint http://host:9090` and a reachable
//!     fakenet. The demo signs a path-2A envelope-only payload, so
//!     `/settle` surfaces `chain_unimplemented` (the surrogate is
//!     populated only by `build_exact_nockchain_payload_with_wallet`,
//!     which the path-2B branch exercises). The demo treats this as
//!     expected and exits 0 — gRPC connectivity itself is the
//!     strongest signal in this mode.
//!   --mode path2b (feature `path2b`): boots a real wallet kernel,
//!     drives `NockchainWalletClient::authorize_and_sign`, and
//!     submits the assembled signed RawTx via the facilitator's
//!     gRPC `/settle`. Operator entry point for row 1 of the
//!     Phase-5B fakenet validation matrix. Requires:
//!       - the harness fakenet running (vesl-agent setup.sh),
//!       - `X402_FAKENET_SIGNING_KEY` exported (8 Belt values),
//!       - the matching PKH funded with a single coinbase note
//!         exactly covering value + fee.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use ibig::UBig;
use nockchain_client_rs::ChainConfig;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use x402_client::{build_exact_nockchain_payload, BazaarClient, X402Client};
use x402_mcp::{
    router_for_registry, McpToolRegistry, PaymentVerifier, RemoteFacilitatorVerifier, ToolHandler,
};
use x402_nockchain_crypto::{NockchainSigner, SchnorrPrivateKey};
use x402_nockchain_facilitator::{
    in_memory_pool, router as facilitator_router, AppState, ChainClient, GrpcChainClient,
    StubChainClient,
};
use x402_types::payment::{PaymentRequirements, PaymentResource};
use x402_types::{ListDiscoveryResourcesParams, McpTransport};

#[derive(Debug, Clone)]
struct Args {
    mode: Mode,
    grpc_endpoint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Stub,
    Grpc,
    /// Path-2B operator entry point. Available only when the binary is
    /// built with `--features path2b`; without that the parser rejects
    /// `--mode path2b` with a clear error.
    Path2b,
}

fn parse_args() -> Args {
    let mut mode = Mode::Stub;
    let mut grpc_endpoint = "http://localhost:9090".to_string();
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--mode" => {
                match iter.next().as_deref() {
                    Some("stub") => mode = Mode::Stub,
                    Some("grpc") => mode = Mode::Grpc,
                    Some("path2b") => {
                        #[cfg(feature = "path2b")]
                        {
                            mode = Mode::Path2b;
                        }
                        #[cfg(not(feature = "path2b"))]
                        {
                            eprintln!(
                                "--mode path2b requires building with `--features path2b` \
                                 (use `bash examples/demo.sh --grpc --path2b`)"
                            );
                            std::process::exit(2);
                        }
                    }
                    other => {
                        eprintln!("--mode expects `stub`, `grpc`, or `path2b` (got {other:?})");
                        std::process::exit(2);
                    }
                }
            }
            "--grpc-endpoint" => {
                if let Some(v) = iter.next() {
                    grpc_endpoint = v;
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown flag: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    Args { mode, grpc_endpoint }
}

fn print_help() {
    println!(
        "demo_full — Phase-4/5 end-to-end x402/MCP demo\n\n\
         Usage: demo_full [--mode stub|grpc|path2b] [--grpc-endpoint URL]\n\n\
         Options:\n\
         \t--mode stub      (default) use StubChainClient. No fakenet needed.\n\
         \t--mode grpc      connect to a real Nockchain node for /settle\n\
         \t                 (path-2A envelope-only; settles via stub bridge).\n\
         \t--mode path2b    boot a wallet kernel + drive\n\
         \t                 NockchainWalletClient. Requires --features path2b.\n\
         \t--grpc-endpoint  URL of the Nockchain public gRPC endpoint\n\
         \t                 (default: http://localhost:9090)\n\n\
         path2b-mode environment:\n\
         \tX402_FAKENET_SIGNING_KEY   8 whitespace-separated Belt values\n\
         \tX402_FAKENET_FIXTURES      path to vesl-agent/harness/fakenet/fixtures/\n\
         \t                           (the demo loads row-1.json — single-output baseline)"
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    println!("[demo] mode = {:?}", args.mode);

    // Observability: structured logs + a /metrics endpoint mounted
    // alongside the facilitator router.
    x402_nockchain_facilitator::init_tracing()?;
    let metrics_handle = x402_nockchain_facilitator::install_metrics_recorder()?;

    // -- Chain client --
    let chain: Arc<dyn ChainClient> = match args.mode {
        Mode::Stub => Arc::new(StubChainClient),
        Mode::Grpc | Mode::Path2b => {
            println!("[demo] connecting gRPC ChainClient at {}", args.grpc_endpoint);
            let grpc = GrpcChainClient::connect(ChainConfig::local(&args.grpc_endpoint))
                .await
                .context(
                    "connect to Nockchain gRPC — is fakenet running? \
                     try `cd ../../vesl-agent && bash harness/fakenet/setup.sh`",
                )?;
            Arc::new(grpc)
        }
    };

    // -- path-2B: pre-load fixture so the MCP server's pay_to / amount /
    // fee are aligned with the funded UTXO. (Stub + grpc modes use the
    // hardcoded demo values from `build_registry`.)
    #[cfg(feature = "path2b")]
    let path2b_fixture: Option<path2b::RowFixture> = if args.mode == Mode::Path2b {
        Some(path2b::load_row1_fixture()?)
    } else {
        None
    };

    // -- Facilitator --
    let pool = in_memory_pool().await.context("boot sqlite")?;
    let catalog: Arc<dyn x402_nockchain_facilitator::CatalogStore> =
        Arc::new(x402_nockchain_facilitator::SqliteCatalogStore::new(pool));
    let state = AppState::new(catalog, chain);
    let facilitator_listener = TcpListener::bind("127.0.0.1:0").await?;
    let facilitator_addr = facilitator_listener.local_addr()?;
    let facilitator_base = format!("http://{}", facilitator_addr);
    let facilitator = facilitator_router(state)
        .merge(x402_nockchain_facilitator::metrics_router(metrics_handle));
    tokio::spawn(async move {
        let _ = axum::serve(facilitator_listener, facilitator).await;
    });
    println!(
        "[demo] facilitator listening on {} (metrics at {}/metrics)",
        facilitator_base, facilitator_base
    );

    // -- MCP server --
    let mcp_listener = TcpListener::bind("127.0.0.1:0").await?;
    let mcp_addr = mcp_listener.local_addr()?;
    let mcp_base = format!("http://{}", mcp_addr);
    let resource_url = format!("{}/mcp/call/echo", mcp_base);

    #[cfg(feature = "path2b")]
    let registry = if let Some(fx) = path2b_fixture.as_ref() {
        build_registry_with_overrides(
            &resource_url,
            &fx.recipient_pkh,
            &fx.value,
            &fx.fee,
        )
    } else {
        build_registry(&resource_url)
    };
    #[cfg(not(feature = "path2b"))]
    let registry = build_registry(&resource_url);
    let verifier: Arc<dyn PaymentVerifier> =
        Arc::new(RemoteFacilitatorVerifier::new(facilitator_base.clone()));
    let mcp_router: Router = router_for_registry(Arc::new(registry), verifier);
    tokio::spawn(async move {
        let _ = axum::serve(mcp_listener, mcp_router).await;
    });
    println!("[demo] mcp server listening on {}", mcp_base);

    let signer = NockchainSigner::new(
        SchnorrPrivateKey::new(UBig::from(123_456_789u64)).expect("demo key"),
    )
    .expect("construct demo signer");

    let http = reqwest::Client::new();

    // -- Client A: unpaid call, expect 402 --
    println!("[demo] step 1: client posts unpaid tool call");
    let resp = http
        .post(&resource_url)
        .json(&json!({ "msg": "hello nockchain" }))
        .send()
        .await
        .context("unpaid POST")?;
    assert_eq!(resp.status().as_u16(), 402, "expected 402");
    let pr: x402_types::payment::PaymentRequired = resp.json().await?;
    assert_eq!(pr.accepts.len(), 1);
    println!("[demo]   received 402 with 1 accepts entry + bazaar extension");

    // -- Sign payload --
    let requirements = pr.accepts[0].clone();
    let payload = match args.mode {
        #[cfg(feature = "path2b")]
        Mode::Path2b => {
            let fx = path2b_fixture
                .as_ref()
                .expect("path2b mode loads fixture above");
            path2b::sign_payload_with_wallet_kernel(&requirements, fx, &pr.extensions).await?
        }
        _ => build_exact_nockchain_payload(&requirements, &signer, &pr.extensions).await?,
    };
    let payload_json = serde_json::to_vec(&payload)?;
    let payment_header =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &payload_json);

    // -- Client A: paid retry, expect 200 --
    println!("[demo] step 2: client retries with signed payload");
    let resp = http
        .post(&resource_url)
        .header(x402_mcp::router::PAYMENT_SIGNATURE_HEADER, payment_header)
        .json(&json!({ "msg": "hello nockchain" }))
        .send()
        .await
        .context("paid POST")?;
    assert_eq!(resp.status().as_u16(), 200, "expected 200 after verify");
    let body: Value = resp.json().await?;
    println!("[demo]   tool returned: {body}");
    assert_eq!(body, json!({ "msg": "hello nockchain" }));

    // -- Settle --
    println!("[demo] step 3: server posts /settle");
    let settle_req = x402_types::facilitator::VerifyRequest {
        payload: payload.clone(),
        requirements: requirements.clone(),
    };
    let settle_resp = http
        .post(format!("{}/settle", facilitator_base))
        .json(&settle_req)
        .send()
        .await?;
    let settle_body: x402_types::facilitator::SettleResponse = settle_resp.json().await?;
    match (settle_body.success, settle_body.transaction, settle_body.error) {
        (true, Some(tx), None) => {
            println!(
                "[demo]   /settle success: tx_id={} status={} block_height={:?}",
                tx.tx_id, tx.status, tx.block_height
            );
        }
        (false, _, Some(err)) if err.code == "chain_unimplemented" => {
            // Path-2A envelope-only flows hit this branch by design.
            // Path-2B should never end up here — `signed_raw_tx` is
            // populated by `build_exact_nockchain_payload_with_wallet`,
            // so the facilitator runs the real submit path.
            if args.mode == Mode::Path2b {
                anyhow::bail!(
                    "[demo] /settle returned chain_unimplemented in path-2B mode — \
                     surrogate was missing from the payload, which means \
                     `build_exact_nockchain_payload_with_wallet` produced an empty \
                     `signedRawTx`. Investigate. message: {}",
                    err.message
                );
            }
            println!(
                "[demo]   /settle reported chain_unimplemented — expected when the\n\
                 [demo]   client builds the path-2A envelope-only payload (this demo's\n\
                 [demo]   default). Per ADR-0010 the facilitator submits the wallet's\n\
                 [demo]   signed RawTx surrogate from `payload.signedRawTx`; that field\n\
                 [demo]   is populated only by `build_exact_nockchain_payload_with_wallet`\n\
                 [demo]   + a real `WalletBackend`. Run `bash examples/demo.sh --grpc \
                 [demo]   --path2b` to exercise the real path.\n\
                 [demo]   message: {}",
                err.message
            );
        }
        other => {
            anyhow::bail!("unexpected /settle response: {other:?}");
        }
    }

    // -- Client B: discovery --
    println!("[demo] step 4: second client queries /discovery/resources?type=mcp");
    let bazaar = BazaarClient::new(facilitator_base.clone());
    let listing = bazaar
        .list_resources(ListDiscoveryResourcesParams {
            kind: Some("mcp".into()),
            limit: Some(10),
            offset: Some(0),
            ..Default::default()
        })
        .await?;
    println!(
        "[demo]   discovery returned {} resource(s); first: {}",
        listing.items.len(),
        listing
            .items
            .first()
            .map(|r| r.resource.clone())
            .unwrap_or_default()
    );
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].kind, "mcp");

    println!("[demo] OK — phase-4 demo completed successfully");
    let _ = X402Client::new();
    Ok(())
}

fn build_registry(resource_url: &str) -> McpToolRegistry {
    build_registry_with_overrides(
        resource_url,
        "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
        "65536",
        "10",
    )
}

#[cfg_attr(not(feature = "path2b"), allow(dead_code))]
fn build_registry_with_overrides(
    resource_url: &str,
    pay_to: &str,
    max_amount_required: &str,
    min_fee: &str,
) -> McpToolRegistry {
    let resource = PaymentResource {
        url: resource_url.to_string(),
        description: Some("Demo MCP echo tool".into()),
        mime_type: Some("application/json".into()),
    };
    let accepts = vec![PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:fakenet".into(),
        max_amount_required: max_amount_required.into(),
        resource: resource_url.to_string(),
        asset: "NOCK".into(),
        pay_to: pay_to.into(),
        max_timeout_seconds: 60,
        description: Some("Demo MCP echo tool".into()),
        mime_type: Some("application/json".into()),
        output_schema: None,
        extra: Some(json!({ "minFee": min_fee })),
        extensions: None,
    }];

    let mut reg = McpToolRegistry::new(resource, accepts);
    let handler: ToolHandler = Arc::new(|args: Value| Box::pin(async move { Ok(args) }));
    reg.register(
        "echo",
        Some("Echo the input back. Demo tool.".into()),
        json!({
            "type": "object",
            "properties": {
                "msg": { "type": "string", "description": "Text to echo back" }
            },
            "required": ["msg"]
        }),
        Some(McpTransport::StreamableHttp),
        handler,
    )
    .expect("register echo");
    reg
}

#[cfg(feature = "path2b")]
mod path2b {
    //! Path-2B branch — boots the wallet kernel, drives
    //! `NockchainWalletClient::authorize_and_sign`, and returns a
    //! `PaymentPayload` with the surrogate populated.
    //!
    //! Single-output exact-cover only: this is the demo equivalent of
    //! row 1 of the Phase-5B fakenet matrix. Multi-seed cases live in
    //! `crates/x402-nockchain-wallet-client/tests/fakenet_path2b.rs`.

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use anyhow::{anyhow, Context, Result};
    use nockapp::kernel::boot;
    use nockchain_client_rs::{ChainClient, ChainConfig};
    use nockchain_math::belt::Belt;
    use serde::Deserialize;
    use serde_json::Value;
    use x402_client::{build_exact_nockchain_payload_with_wallet, WalletBackend};
    use x402_nockchain_wallet_client::{NockchainWalletClient, WalletKernelHandle};
    use x402_types::payment::{NoteLock, NoteName, NoteRef, PaymentPayload, PaymentRequirements};

    #[derive(Debug, Clone, Deserialize)]
    #[allow(dead_code)] // `label`/`change_pkh` carried for fixture-format compatibility
    pub struct RowFixture {
        #[serde(default)]
        pub label: String,
        pub grpc_endpoint: String,
        pub payer_pkh: String,
        pub recipient_pkh: String,
        pub change_pkh: String,
        pub input_note: FixtureNote,
        pub value: String,
        pub fee: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct FixtureNote {
        pub name_first: String,
        pub name_last: String,
        pub assets: String,
        /// Lock the UTXO is held under (Phase-6 lift). Defaults to
        /// `simple-pkh` for backwards-compatible fixtures; coinbase
        /// inputs set `{kind: "coinbase-pkh", timelock_min: 1}` for
        /// fakenet.
        #[serde(default)]
        pub lock: NoteLock,
    }

    pub fn load_row1_fixture() -> Result<RowFixture> {
        let dir = std::env::var("X402_FAKENET_FIXTURES").map_err(|_| {
            anyhow!(
                "path-2B mode needs X402_FAKENET_FIXTURES pointing at \
                 vesl-agent/harness/fakenet/fixtures/. See the operator runbook."
            )
        })?;
        let path = PathBuf::from(dir).join("row-1.json");
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read fixture {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("parse fixture {}", path.display()))
    }

    fn load_signing_key() -> Result<[Belt; 8]> {
        let raw = std::env::var("X402_FAKENET_SIGNING_KEY").map_err(|_| {
            anyhow!(
                "path-2B mode needs X402_FAKENET_SIGNING_KEY (8 Belt values, \
                 whitespace-separated). Source it from \
                 vesl-agent/harness/fakenet/funded-keys/<key>.t8."
            )
        })?;
        let parts: Vec<u64> = raw
            .split_whitespace()
            .map(|s| s.parse::<u64>().map_err(|e| anyhow!("Belt {s}: {e}")))
            .collect::<Result<_>>()?;
        if parts.len() != 8 {
            return Err(anyhow!(
                "X402_FAKENET_SIGNING_KEY must contain exactly 8 Belt values, got {}",
                parts.len()
            ));
        }
        Ok([
            Belt(parts[0]),
            Belt(parts[1]),
            Belt(parts[2]),
            Belt(parts[3]),
            Belt(parts[4]),
            Belt(parts[5]),
            Belt(parts[6]),
            Belt(parts[7]),
        ])
    }

    pub async fn sign_payload_with_wallet_kernel(
        requirements: &PaymentRequirements,
        fixture: &RowFixture,
        echo_extensions: &Option<BTreeMap<String, Value>>,
    ) -> Result<PaymentPayload<Value>> {
        let sk = load_signing_key()?;
        let chain = ChainClient::connect(ChainConfig::local(&fixture.grpc_endpoint))
            .await
            .with_context(|| format!("connect chain at {}", fixture.grpc_endpoint))?;

        // Boot a fresh kernel under a temp data dir. The kernel must
        // expose %sig-hash + %tx-id pokes at its top-level poke arm —
        // load the JAM from `X402_KERNEL_JAM` (e.g. vesl.jam). The
        // open-wallet kernel does not qualify; see the runbook.
        let kernel_path = std::env::var("X402_KERNEL_JAM").map_err(|_| {
            anyhow!(
                "X402_KERNEL_JAM must point at a kernel JAM exposing \
                 %sig-hash + %tx-id pokes (e.g. hull-llm/assets/vesl.jam). \
                 See vesl-agent/harness/fakenet/runbook.md."
            )
        })?;
        let kernel_bytes = std::fs::read(&kernel_path)
            .with_context(|| format!("read kernel JAM at {kernel_path}"))?;
        let data_dir = std::env::current_dir()?.join(".path2b-wallet-state");
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).context("create wallet state dir")?;
        let cli = boot::default_boot_cli(true);
        let app = boot::setup(&kernel_bytes, cli, &[], "wallet", Some(data_dir.clone()))
            .await
            .map_err(|e| anyhow!("boot kernel from {kernel_path}: {e}"))?;
        let kernel = WalletKernelHandle::from_app(app);

        let wallet = NockchainWalletClient::from_signing_key(chain, kernel, sk)
            .map_err(|e| anyhow!("construct wallet client: {e}"))?;

        // Fixture's payer_pkh is the chain-form PKH (what coinbase
        // notes are locked under). The wallet's `payer_pkh()` is the
        // envelope-form PKH (used in `auth.from`). Two distinct
        // hashes of the same key — see ADR-0017.
        if wallet.payer_pkh_chain_b58() != fixture.payer_pkh {
            return Err(anyhow!(
                "fixture payer_pkh ({}) does not match the signing-key-derived chain PKH ({}). \
                 Regenerate funded keys + fixtures together (see runbook).",
                fixture.payer_pkh,
                wallet.payer_pkh_chain_b58()
            ));
        }

        let notes = vec![NoteRef {
            lock: fixture.input_note.lock.clone(),
            name: NoteName {
                first: fixture.input_note.name_first.clone(),
                last: fixture.input_note.name_last.clone(),
            },
            assets: fixture.input_note.assets.clone(),
        }];

        println!(
            "[demo]   path-2B: payer={} recipient={} value={} fee={}",
            wallet.payer_pkh(),
            fixture.recipient_pkh,
            fixture.value,
            fixture.fee
        );

        let payload = build_exact_nockchain_payload_with_wallet(
            requirements,
            &wallet,
            notes,
            echo_extensions,
        )
        .await
        .context("build_exact_nockchain_payload_with_wallet")?;
        Ok(payload)
    }
}
