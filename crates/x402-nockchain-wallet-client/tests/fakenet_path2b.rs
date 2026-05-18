//! Fakenet path-2B validation matrix (Phase 5B).
//!
//! Six `#[tokio::test]` + `#[ignore]` rows that drive
//! `NockchainWalletClient::authorize_and_sign` end-to-end against a
//! running Nockchain fakenet. Tests retire the residual risk flagged at
//! Phase 4 close — Rust-side noun round-trips are unit-tested but the
//! kernel's Hoon-side decoder behaviour against a real `wal.jam` is
//! unverified until this matrix runs.
//!
//! These tests are off by default in two ways:
//!
//!   1. The whole file is gated behind the `_wallet_kernel_tests`
//!      Cargo feature; without that feature the test crate compiles to
//!      nothing.
//!   2. Every test is `#[ignore]`-d so even with the feature on a
//!      bare `cargo test` keeps skipping them. The operator must pass
//!      `-- --ignored --test-threads=1` (the matrix is intentionally
//!      sequential — see the per-row notes below).
//!
//! ## Operator entry point
//!
//! Bring up the fakenet harness from the proprietary repo
//! (`vesl-agent/harness/fakenet/setup.sh`) so the chain is mining and
//! the demo PKH has the right UTXO shapes, then:
//!
//! ```bash
//! cargo test -p x402-nockchain-wallet-client \
//!     --features _wallet_kernel_tests --test fakenet_path2b \
//!     -- --ignored --test-threads=1
//! ```
//!
//! ## Sequencing
//!
//! Rows are strictly ordered. Row 1 (single-output baseline) is the
//! minimum bar; if it fails, the multi-seed cases are pointless and the
//! `authorize_and_sign` change-output lift must be reconsidered. Stop
//! at the first failure and capture the kernel-side error in ADR-0017.
//!
//! ## Inputs
//!
//! Per-row inputs live in
//! `vesl-agent/harness/fakenet/fixtures/row-{1..6}.json` (operator
//! repo). Tests load fixture content from the path supplied via
//! `X402_FAKENET_FIXTURES`. Each fixture pins UTXO names + amounts
//! that the harness `setup.sh` carved out of mined coinbase. Re-running
//! the matrix without resetting the harness will fail row 1 because the
//! input UTXOs are gone — that's a state-drift symptom, not a code
//! regression. See `harness/fakenet/runbook.md` for the reset recipe.

#![cfg(feature = "_wallet_kernel_tests")]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use nockapp::kernel::boot;
use nockapp::NockApp;
use nockchain_client_rs::{ChainClient, ChainConfig};
use nockchain_math::belt::Belt;
use serde::Deserialize;
use tempfile::TempDir;
use x402_client::{generate_nonce_base58, WalletBackend};
use x402_nockchain_wallet_client::{NockchainWalletClient, WalletKernelHandle};
use x402_types::payment::{Authorization, NoteLock, NoteName, NoteRef, PaymentRequirements};

// ---------------------------------------------------------------------------
// Fixture shape
// ---------------------------------------------------------------------------

/// Per-row fixture loaded from
/// `vesl-agent/harness/fakenet/fixtures/row-N.json`. The harness
/// `setup.sh` writes this after mining + splitting UTXOs.
#[derive(Debug, Clone, Deserialize)]
struct RowFixture {
    /// Human-readable row label (e.g. "row-1: single-output exact-cover").
    label: String,
    /// gRPC endpoint of the running fakenet.
    grpc_endpoint: String,
    /// Base58-encoded payer PKH (the harness mines coinbase to this).
    payer_pkh: String,
    /// Base58-encoded recipient PKH.
    recipient_pkh: String,
    /// Base58-encoded change PKH (may equal `payer_pkh` for row 3).
    change_pkh: String,
    /// The single input note the matrix spends in this row.
    input_note: FixtureNote,
    /// Payment value in nicks (decimal string).
    value: String,
    /// Fee in nicks (decimal string).
    fee: String,
    /// Path-2B expected outcome — `Accept` for rows 1–5, `Reject` for
    /// row 6 (the negative test mutates the surrogate before submit).
    #[serde(default)]
    expected_outcome: Outcome,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureNote {
    name_first: String,
    name_last: String,
    /// Total assets in this UTXO (decimal string in nicks). Must satisfy
    /// `assets >= value + fee` for accept rows.
    assets: String,
    /// Lock type the UTXO is held under. Defaults to `simple-pkh` for
    /// pre-Phase-6 fixtures; the Phase-6 matrix populates this with
    /// `{kind: "coinbase-pkh", timelock_min: <chain_constant>}`
    /// because the harness spends mined coinbase notes directly. The
    /// timelock value comes from
    /// `nockchain_types::blockchain_constants::fakenet_blockchain_constants`
    /// (`with_coinbase_timelock_min(1)`).
    #[serde(default)]
    lock: NoteLock,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    #[default]
    Accept,
    /// Negative test (row 6) — the chain must reject the submitted
    /// transaction after the test mutates `seeds[1].lock_root` post-sign.
    Reject,
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load a row's fixture from disk. Path comes from `X402_FAKENET_FIXTURES`,
/// which the operator points at `vesl-agent/harness/fakenet/fixtures/`.
fn load_fixture(row: u8) -> Result<RowFixture> {
    let dir = std::env::var("X402_FAKENET_FIXTURES").map_err(|_| {
        anyhow!(
            "X402_FAKENET_FIXTURES must point at vesl-agent/harness/fakenet/fixtures/. \
             See vesl-agent/harness/fakenet/runbook.md for setup."
        )
    })?;
    let path = PathBuf::from(dir).join(format!("row-{}.json", row));
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read fixture {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse fixture {}", path.display()))
}

/// Funded payer signing key as `[Belt; 8]` (Hoon `t8` layout).
///
/// Loaded from `X402_FAKENET_SIGNING_KEY` — eight whitespace-separated
/// decimal Belt values. The operator harness writes this from
/// `vesl-agent/harness/fakenet/funded-keys/<key>.t8` after generating
/// the funded identity.
fn load_signing_key() -> Result<[Belt; 8]> {
    let raw = std::env::var("X402_FAKENET_SIGNING_KEY").map_err(|_| {
        anyhow!(
            "X402_FAKENET_SIGNING_KEY must hold the t8 Belt array of the funded payer. \
             See vesl-agent/harness/fakenet/runbook.md."
        )
    })?;
    let parts: Vec<u64> = raw
        .split_whitespace()
        .map(|s| s.parse::<u64>().map_err(|e| anyhow!("parse Belt {s}: {e}")))
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

/// Boot a fresh `NockApp` from a kernel JAM file at the path supplied
/// in `X402_KERNEL_JAM`. The kernel must expose `%sig-hash` and
/// `%tx-id` pokes at its top-level poke arm — `vesl-core/protocol/lib/
/// vesl-kernel.hoon` (compiled to `vesl.jam`) is the canonical fit.
/// The open-wallet kernel (`wal.jam`) does *not* qualify: it lacks
/// these pokes at its top level, returning "input does not have a
/// proper cause" against the matrix's poke shape.
///
/// Returns the booted `NockApp` plus the `TempDir` it stores its state
/// under (kept alive for the test's lifetime — dropping it deletes the
/// state directory).
async fn boot_wallet_kernel() -> Result<(NockApp, TempDir)> {
    let kernel_path = std::env::var("X402_KERNEL_JAM").map_err(|_| {
        anyhow!(
            "X402_KERNEL_JAM must point at a kernel JAM exposing %sig-hash + %tx-id pokes \
             (e.g. hull-llm/assets/vesl.jam). See vesl-agent/harness/fakenet/runbook.md."
        )
    })?;
    let kernel_bytes = std::fs::read(&kernel_path)
        .with_context(|| format!("read kernel JAM at {kernel_path}"))?;
    let tmp = tempfile::tempdir().context("create wallet tempdir")?;
    let cli = boot::default_boot_cli(true);
    let app = boot::setup(&kernel_bytes, cli, &[], "wallet", Some(tmp.path().to_path_buf()))
        .await
        .map_err(|e| anyhow!("boot kernel from {kernel_path}: {e}"))?;
    Ok((app, tmp))
}

/// Common setup: load fixture + signing key, boot kernel, connect chain,
/// construct `NockchainWalletClient`. Returns the constructed client
/// and the kernel-state tempdir (kept alive by the caller).
async fn setup(row: u8) -> Result<(RowFixture, NockchainWalletClient, TempDir)> {
    let fixture = load_fixture(row)?;
    let sk = load_signing_key()?;

    let (app, tmp) = boot_wallet_kernel().await?;
    let kernel = WalletKernelHandle::from_app(app);

    let chain = ChainClient::connect(ChainConfig::local(&fixture.grpc_endpoint))
        .await
        .with_context(|| format!("connect chain at {}", fixture.grpc_endpoint))?;
    let client = NockchainWalletClient::from_signing_key(chain, kernel, sk)
        .map_err(|e| anyhow!("construct wallet client: {e}"))?;

    // The fixture's `payer_pkh` is the *chain-form* PKH (the form
    // coinbase notes are locked under). The wallet exposes that
    // separately from the envelope-form PKH that `auth.from` uses.
    if client.payer_pkh_chain_b58() != fixture.payer_pkh {
        return Err(anyhow!(
            "fixture payer_pkh ({}) does not match the signing-key-derived chain PKH ({}). \
             The funded key and the fixture were generated against different identities — \
             regenerate per the runbook.",
            fixture.payer_pkh,
            client.payer_pkh_chain_b58()
        ));
    }
    Ok((fixture, client, tmp))
}

/// Build the `Authorization` for a row from its fixture. `auth.from`
/// gets the **envelope-form** PKH so the §5.4.1 verifier passes; the
/// fixture's `payer_pkh` is the **chain-form** PKH (used elsewhere in
/// the matrix to assert key/coinbase alignment). The two are
/// different hashes of the same key (Phase-5B finding).
fn auth_from_fixture(fixture: &RowFixture, client: &NockchainWalletClient) -> Authorization {
    let now = now_secs();
    Authorization {
        from: client.payer_pkh(),
        to: fixture.recipient_pkh.clone(),
        value: fixture.value.clone(),
        fee: fixture.fee.clone(),
        nonce: generate_nonce_base58(),
        valid_after: now.saturating_sub(5),
        valid_before: now.saturating_add(120),
        notes: vec![NoteRef {
            name: NoteName {
                first: fixture.input_note.name_first.clone(),
                last: fixture.input_note.name_last.clone(),
            },
            assets: fixture.input_note.assets.clone(),
            // Phase 6: input lock metadata flows through unchanged.
            // Coinbase-locked UTXOs need a `coinbase_pkh` witness;
            // see `NoteLock` + ADR-0017 finding #3.
            lock: fixture.input_note.lock.clone(),
        }],
        change_address: fixture.change_pkh.clone(),
    }
}

/// Synthetic `PaymentRequirements` paired with the fixture's
/// authorization. The wallet's `authorize_and_sign` ignores the
/// requirements (the spec digest is over `auth` alone); any consistent
/// value satisfies the §5.4.1 envelope check the matrix exercises.
fn requirements_from_auth(auth: &Authorization) -> PaymentRequirements {
    PaymentRequirements {
        scheme: "exact".into(),
        network: "nockchain:fakenet".into(),
        max_amount_required: auth.value.clone(),
        resource: "fakenet-matrix://row".into(),
        asset: "NOCK".into(),
        pay_to: auth.to.clone(),
        max_timeout_seconds: 60,
        description: None,
        mime_type: None,
        output_schema: None,
        extra: None,
        extensions: None,
    }
}

/// Submit the wallet-signed `RawTx` surrogate via the chain client and
/// assert acceptance. `submit_and_wait` returns `Ok(true)` on
/// acceptance, `Ok(false)` on poll-timeout — we treat poll-timeout as a
/// fakenet-availability problem and surface it as an error so it's
/// distinguishable from "kernel produced bytes the chain rejected."
async fn submit_and_assert_accepted(
    client: &NockchainWalletClient,
    auth: &Authorization,
) -> Result<()> {
    let requirements = requirements_from_auth(auth);
    let signed = client
        .authorize_and_sign(auth, &requirements)
        .await
        .map_err(|e| anyhow!("authorize_and_sign: {e}"))?;
    let tx_id = signed.signed_raw_tx.tx_id.clone();
    eprintln!("[submit] tx_id={tx_id}");
    let raw = x402_nockchain_wallet_client::raw_tx_from_surrogate(&signed.signed_raw_tx)
        .context("decode surrogate")?;
    let chain = client.chain();
    let mut chain = chain.lock().await;
    let accepted = chain
        .submit_and_wait(raw, &tx_id)
        .await
        .context("submit_and_wait — RPC error")?;
    if !accepted {
        return Err(anyhow!(
            "submit_and_wait timed out without acceptance for tx_id {tx_id} \
             — fakenet may be stalled or the tx silently dropped"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Matrix (rows are strictly ordered — stop at the first failure)
// ---------------------------------------------------------------------------

/// Row 1 — single-input, single-output, exact-cover (no change seed).
///
/// Byte-equivalent of the Phase-4 close baseline. Validates the basic
/// pipeline: kernel boot → `%sig-hash` → Rust sign → `%tx-id` → submit
/// → wait → accepted. If this row fails, multi-seed rows are pointless
/// and the close commit's `authorize_and_sign` lift needs reconsidering
/// before retrying.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fakenet path-2B matrix — operator-only; run with --ignored"]
async fn row_1_single_output_exact_cover() -> Result<()> {
    let (fixture, client, _tmp) = setup(1).await?;
    assert_eq!(fixture.expected_outcome, Outcome::Accept);
    let auth = auth_from_fixture(&fixture, &client);
    submit_and_assert_accepted(&client, &auth).await?;
    eprintln!("[row-1] {}: accepted", fixture.label);
    Ok(())
}

/// Row 2 — single-input, two-output (payment + change), change to a
/// different PKH than the payer.
///
/// Primary lifted case from the Phase-4 close. Verifies the kernel's
/// Hoon-side decoder accepts the canonical multi-seed `Seeds::to_noun`
/// jam. If row 2 fails, revert the change-output lift in
/// `authorize_and_sign` to single-output-only and capture the failure
/// shape in ADR-0017.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fakenet path-2B matrix — operator-only; run with --ignored"]
async fn row_2_two_output_change_to_different_pkh() -> Result<()> {
    let (fixture, client, _tmp) = setup(2).await?;
    assert_eq!(fixture.expected_outcome, Outcome::Accept);
    assert_ne!(
        fixture.change_pkh, fixture.payer_pkh,
        "row-2 fixture must point change at a non-payer PKH"
    );
    let auth = auth_from_fixture(&fixture, &client);
    submit_and_assert_accepted(&client, &auth).await?;
    eprintln!("[row-2] {}: accepted", fixture.label);
    Ok(())
}

/// Row 3 — single-input, two-output, change *back to payer's own PKH*.
///
/// The common UX shape (most wallets default to self-change). Catches
/// any kernel-side aliasing assumption between input PKH and output PKH
/// lock roots.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fakenet path-2B matrix — operator-only; run with --ignored"]
async fn row_3_two_output_change_to_self() -> Result<()> {
    let (fixture, client, _tmp) = setup(3).await?;
    assert_eq!(fixture.expected_outcome, Outcome::Accept);
    assert_eq!(
        fixture.change_pkh, fixture.payer_pkh,
        "row-3 fixture must point change at the payer's own PKH"
    );
    let auth = auth_from_fixture(&fixture, &client);
    submit_and_assert_accepted(&client, &auth).await?;
    eprintln!("[row-3] {}: accepted", fixture.label);
    Ok(())
}

/// Row 4 — change is exactly 1 nick (minimum non-zero amount).
///
/// Probes the chain's dust-threshold behaviour. If the chain rejects
/// sub-dust outputs, the wallet must either reject in-place or coalesce
/// dust into the fee — both options get captured in ADR-0017.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fakenet path-2B matrix — operator-only; run with --ignored"]
async fn row_4_one_nick_change() -> Result<()> {
    let (fixture, client, _tmp) = setup(4).await?;
    assert_eq!(fixture.expected_outcome, Outcome::Accept);
    let auth = auth_from_fixture(&fixture, &client);
    let value: u64 = auth.value.parse()?;
    let fee: u64 = auth.fee.parse()?;
    let assets: u64 = auth.notes[0].assets.parse()?;
    assert_eq!(
        assets - value - fee,
        1,
        "row-4 fixture must produce exactly 1 nick of change"
    );
    submit_and_assert_accepted(&client, &auth).await?;
    eprintln!("[row-4] {}: accepted", fixture.label);
    Ok(())
}

/// Row 5 — payment value is 1 nick, change carries the bulk.
///
/// Inverse of row 4. Catches asymmetric handling between primary and
/// change seeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fakenet path-2B matrix — operator-only; run with --ignored"]
async fn row_5_one_nick_payment() -> Result<()> {
    let (fixture, client, _tmp) = setup(5).await?;
    assert_eq!(fixture.expected_outcome, Outcome::Accept);
    let auth = auth_from_fixture(&fixture, &client);
    assert_eq!(auth.value, "1", "row-5 fixture must set value to 1 nick");
    submit_and_assert_accepted(&client, &auth).await?;
    eprintln!("[row-5] {}: accepted", fixture.label);
    Ok(())
}

/// Row 6 — chain rejects a tampered change seed (negative test).
///
/// We sign normally, then mutate the change seed's `lock_root` on the
/// surrogate before submission. The Hoon-side validator should detect
/// the signature/seeds mismatch and reject the tx — confirming the
/// binding between `sig_hash` and the actual submitted `Spends` still
/// holds at the chain layer.
///
/// If this row's mutation submits cleanly, we have a real binding
/// regression. Capture the failure shape in ADR-0017 and stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fakenet path-2B matrix — operator-only; run with --ignored"]
async fn row_6_tampered_change_seed_rejected() -> Result<()> {
    let (fixture, client, _tmp) = setup(6).await?;
    assert_eq!(fixture.expected_outcome, Outcome::Reject);
    let auth = auth_from_fixture(&fixture, &client);

    let requirements = requirements_from_auth(&auth);
    let signed = client
        .authorize_and_sign(&auth, &requirements)
        .await
        .map_err(|e| anyhow!("authorize_and_sign: {e}"))?;
    eprintln!("[submit] tx_id={} (pre-tamper)", signed.signed_raw_tx.tx_id);
    let mut surrogate = signed.signed_raw_tx.clone();

    // Locate the single spend's seeds and mutate seeds[1].lock_root
    // (the change seed). The mutation is intentionally noisy in test
    // logs — silent skip would let a real binding regression land.
    let entry = surrogate
        .spends
        .first_mut()
        .ok_or_else(|| anyhow!("no spend in surrogate"))?;
    let seed_count = entry.spend.seeds.len();
    if seed_count < 2 {
        return Err(anyhow!(
            "row-6 expected 2 seeds (payment + change), got {seed_count} — \
             fixture must produce a change seed"
        ));
    }
    let original = entry.spend.seeds[1].lock_root.clone();
    // Replace with a syntactically-valid base58 hash that does not
    // match the signed lock_root. "11111111111111111111111111111111"
    // is base58 for 32 zero bytes — guaranteed to differ from any
    // real Tip5 hash the wallet would have signed.
    entry.spend.seeds[1].lock_root = "11111111111111111111111111111111".into();
    eprintln!(
        "[row-6] mutated seeds[1].lock_root: was {original}, now zeros-base58"
    );

    let tx_id = surrogate.tx_id.clone();
    let raw = x402_nockchain_wallet_client::raw_tx_from_surrogate(&surrogate)
        .context("decode mutated surrogate")?;
    let chain = client.chain();
    let mut chain = chain.lock().await;
    let outcome = chain.submit_and_wait(raw, &tx_id).await;
    match outcome {
        Ok(true) => Err(anyhow!(
            "[row-6] FAIL: chain accepted the tampered tx — \
             sig_hash/seeds binding regression at the chain layer"
        )),
        Ok(false) => {
            eprintln!(
                "[row-6] {}: chain did not accept tampered tx within timeout (rejected/dropped)",
                fixture.label
            );
            Ok(())
        }
        Err(e) => {
            eprintln!(
                "[row-6] {}: submit raised error as expected: {e}",
                fixture.label
            );
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-references (kept here so a fresh reader of this file lands on
// the right docs without grep)
// ---------------------------------------------------------------------------
//
// - `vesl-agent/docs/plans/phase-5b-fakenet-path2b-validation.md` — the
//   plan this file implements.
// - `vesl-agent/harness/fakenet/runbook.md` — operator runbook.
// - `x402-nockchain/docs/decisions/0017-fakenet-path2b-validation.md` —
//   closing ADR (per-row pass/fail recorded there after a real run).
// - `x402-nockchain/docs/decisions/0010-path-2b-client-assembled-rawtx.md` —
//   why path-2B exists at all.
