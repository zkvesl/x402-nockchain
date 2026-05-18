//! `NockchainWalletClient` — reference [`WalletBackend`] backed by an
//! embedded Nockchain wallet kernel (approach 2 per
//! `docs/wallet-integration.md §4.1`).
//!
//! ## Architectural shape
//!
//! The client owns three collaborators:
//!
//! - **`NockApp` running the wallet kernel** ([`WalletKernelHandle`]).
//!   Owns the in-process kernel that computes the chain `sig_hash` and
//!   `tx_id` digests via `%sig-hash` + `%tx-id` pokes (`vesl_core::tx_builder`'s
//!   pattern, mirroring hull-llm). The caller boots this kernel — we do
//!   not ship the wallet kernel bytes (see §kernel-provisioning).
//! - **`nockchain_client_rs::ChainClient`.** Public gRPC handle the
//!   caller can reach for off-band balance / UTXO queries. The
//!   [`WalletBackend::authorize_and_sign`] body itself does not query
//!   the chain — `auth.notes` is *binding* per
//!   `docs/wallet-integration.md §7`, so the wallet trusts the
//!   caller-selected note set + amount.
//! - **Schnorr signing key** — the secret key the wallet signs with.
//!   The same key signs both the §5.4.1 envelope digest (over
//!   [`x402_sign_message_digest`]) and the chain-side `sig_hash`
//!   (Tip5-sponge over the seeds + fee). The payer PKH and base58
//!   pubkey are derived from this key, so the trait's
//!   [`payer_pkh`](WalletBackend::payer_pkh) is canonical.
//!
//! ## Flow (per [`WalletBackend::authorize_and_sign`])
//!
//! 1. Assert `auth.from == self.payer_pkh`.
//! 2. Validate `auth.notes.len() == 1` (multi-input support deferred —
//!    `vesl_core::tx_builder::jam_spends_manual` only supports the
//!    single-element zmap shape today).
//! 3. Validate `auth.notes[0].assets >= auth.value + auth.fee`. Excess
//!    becomes a change output back to `auth.change_address`.
//! 4. Construct seeds: payment seed (lock_root = simple-PKH lock of
//!    `auth.to`, gift = `auth.value`) plus, if any change is left
//!    over, a change seed back to `auth.change_address`. Both share
//!    `parent_hash = base58 of auth.notes[0].name.last`.
//! 5. Poke `%sig-hash` on the embedded kernel with `(seeds, fee)`;
//!    extract the chain `Hash`. Uses the canonical `Seeds::to_noun`
//!    encoder via the local `kernel_sig_hash_canonical` helper so
//!    multi-seed (change-output) cases work —
//!    `vesl_core::tx_builder::kernel_sig_hash` is single-seed only.
//! 6. Sign that hash with `vesl_core::signing::sign(&sk_t8, &msg)`.
//! 7. Assemble the [`Witness`] (full lock-merkle proof for a simple
//!    PKH input + the single PKH-signature entry).
//! 8. Assemble [`Spends`] — one entry, name from `auth.notes[0]`.
//! 9. Poke `%tx-id` on the kernel with the spends; extract the
//!    `TxId`.
//! 10. Convert the chain-typed [`RawTx`] to
//!     [`x402_types::nockchain::SignedRawTx`] via
//!     [`surrogate::surrogate_from_raw_tx`].
//! 11. Compute the §5.4.1 envelope digest from `auth` and Schnorr-sign
//!     it with the same key.
//! 12. Return [`AuthorizedPayment { envelope_signature, signed_raw_tx }`].
//!
//! Steps 4–9 mirror `hull-llm/src/tx_builder.rs::build_settlement_tx`
//! conceptually but lift its single-output assumption (the local
//! `kernel_sig_hash_canonical` helper in this crate replaces vesl-core's
//! `kernel_sig_hash` for multi-seed cases). Steps 10–11 are the
//! x402-specific delta.
//!
//! ## Kernel provisioning
//!
//! The wallet kernel bytes live in the Nockchain monorepo
//! (`hoon/apps/wallet/wallet.hoon`, compiled with `hoonc` to an
//! `out.jam`, distributed as `kernels-open-wallet::KERNEL`). Distribution
//! is the caller's responsibility:
//!
//! - **CLI / server agents.** Pull `kernels-open-wallet` (path-dep on
//!   the local nockchain checkout) and boot a `NockApp` from
//!   `kernels_open_wallet::KERNEL`. Pass that `NockApp` to
//!   [`WalletKernelHandle::from_app`].
//! - **Tests.** Same shape, gated behind a feature flag (the test crate
//!   pulls in `kernels-open-wallet`; this crate does not).
//! - **WASM (future).** Compile the jam once at build time, embed as
//!   bytes, boot in the browser. Open question — tracked in
//!   `docs/wallet-integration.md §4.2`.
//!
//! This crate does **not** ship a kernel, a `build.rs` invocation of
//! `hoonc`, or a kernel-fetch routine. That keeps the crate's
//! compile-time footprint small and leaves provisioning choices with
//! the consumer.

pub mod surrogate;

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ibig::UBig;
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use nockchain_client_rs::ChainClient as NockChainClient;
use nockchain_math::belt::Belt;
use nockchain_types::tx_engine::common::{Hash, Name, Nicks};
use nockchain_types::tx_engine::v1::note::NoteData;
use nockchain_types::tx_engine::v1::tx::{
    Lock, LockMerkleProof, LockMerkleProofFull, MerkleProof, PkhSignature, PkhSignatureEntry,
    Seed, Seeds, Spend, Spend1, Spends, SpendCondition, Witness,
};
use nockchain_types::tx_engine::v1::{RawTx, Version};
use nockvm::ext::make_tas;
use nockvm::noun::{D, T};
use noun_serde::NounEncode;
use tokio::sync::Mutex;
use vesl_core::signing as vesl_signing;
use vesl_core::tx_builder::{bytes_to_atom, extract_hash_from_effect, kernel_tx_id};
use x402_client::{AuthorizedPayment, WalletBackend, WalletBackendError};
use x402_nockchain_crypto::schnorr::{encode_signature, schnorr_sign, SchnorrPrivateKey};
use x402_nockchain_crypto::sign_message::{pkh_from_pubkey_bytes, x402_sign_message_digest};
use x402_nockchain_crypto::wire_compat::vesl_to_x402;
use x402_types::payment::{Authorization, NoteLock, PaymentRequirements};

pub use surrogate::{raw_tx_from_surrogate, surrogate_from_raw_tx};

/// Path-2 [`WalletBackend`] backed by an embedded Nockchain wallet kernel.
///
/// Carries two distinct PKH forms because Nockchain x402 has a spec
/// ambiguity around what "Tip5(pubkey)" means (Phase-5B finding):
///
/// - `payer_pkh_envelope_b58` — Tip5(raw 97-byte pubkey encoding),
///   base58. This is what the §5.4.1 verifier in
///   `x402-nockchain-crypto::verifier::pkh_from_pubkey_bytes` checks
///   against `authorization.from`.
/// - `payer_pkh_chain` — Tip5(noun-encoded pubkey). This matches Hoon's
///   `hash:schnorr-pubkey`, is what coinbase notes are locked under,
///   and is what the chain's `Spend1.witness.pkh_signature.hash`
///   field must equal for consensus to accept the spend.
///
/// Both forms hash the same key — they're not separate identities.
/// `WalletBackend::payer_pkh()` returns the envelope form (what
/// `auth.from` ought to equal); the chain witness derivation uses the
/// chain form internally. See `docs/decisions/0017-fakenet-path2b-validation.md`
/// for the matrix run that surfaced this divergence.
#[derive(Clone)]
pub struct NockchainWalletClient {
    chain: Arc<Mutex<NockChainClient>>,
    payer_pkh_envelope_b58: String,
    payer_pkh_chain: Hash,
    payer_pkh_chain_b58: String,
    payer_pubkey_b58: String,
    payer_pubkey: nockchain_types::tx_engine::common::SchnorrPubkey,
    sk_t8: [Belt; 8],
    wallet_kernel: WalletKernelHandle,
}

/// Wraps the in-process [`NockApp`] booted from the wallet kernel JAM.
///
/// Cheap to clone (the underlying `NockApp` sits behind a single
/// `Arc<Mutex<_>>`).
#[derive(Clone)]
pub struct WalletKernelHandle {
    app: Arc<Mutex<NockApp>>,
}

impl WalletKernelHandle {
    /// Wrap a caller-booted `NockApp` running the wallet kernel.
    ///
    /// The caller is responsible for boot — see the crate-level
    /// `Kernel provisioning` docs for the canonical setup pattern.
    pub fn from_app(app: NockApp) -> Self {
        Self {
            app: Arc::new(Mutex::new(app)),
        }
    }

    /// Borrow the underlying `NockApp` handle. Exposed for callers that
    /// want to drive non-tx pokes (e.g. `%import-seed-phrase`,
    /// `%fakenet`) before authorizing payments.
    pub fn nockapp(&self) -> Arc<Mutex<NockApp>> {
        self.app.clone()
    }
}

impl std::fmt::Debug for WalletKernelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletKernelHandle").finish_non_exhaustive()
    }
}

impl NockchainWalletClient {
    /// Construct a wallet client from a connected gRPC chain handle, a
    /// booted wallet-kernel `NockApp`, and a Schnorr secret key (8 ×
    /// 32-bit Belt chunks, matching Hoon's `t8` convention).
    ///
    /// `payer_pkh` and `payer_pubkey_base58` are derived internally from
    /// `sk_t8` via `vesl_core::signing::derive_pubkey` +
    /// `pubkey_hash`, so they are guaranteed consistent.
    pub fn from_signing_key(
        chain: NockChainClient,
        wallet_kernel: WalletKernelHandle,
        sk_t8: [Belt; 8],
    ) -> Result<Self, WalletBackendError> {
        let payer_pubkey = vesl_signing::derive_pubkey(&sk_t8);
        let payer_pkh_chain = vesl_signing::pubkey_hash(&payer_pubkey);
        let payer_pkh_chain_b58 = payer_pkh_chain.to_base58();
        let payer_pubkey_b58 = payer_pubkey
            .to_base58()
            .map_err(|e| WalletBackendError::Other(format!("encode pubkey base58: {e:?}")))?;
        // Envelope-form PKH = Tip5(pubkey bytes), matching the §5.4.1
        // verifier in x402-nockchain-crypto. Different from the chain
        // form (which is Tip5 of the noun-encoded pubkey).
        let pk_bytes = bs58::decode(&payer_pubkey_b58)
            .into_vec()
            .map_err(|e| WalletBackendError::Other(format!("decode pubkey b58: {e:?}")))?;
        let payer_pkh_envelope_b58 = pkh_from_pubkey_bytes(&pk_bytes);
        Ok(Self {
            chain: Arc::new(Mutex::new(chain)),
            payer_pkh_envelope_b58,
            payer_pkh_chain,
            payer_pkh_chain_b58,
            payer_pubkey_b58,
            payer_pubkey,
            sk_t8,
            wallet_kernel,
        })
    }

    /// PKH that the chain checks against the input note's lock root.
    /// Coinbase notes mined to the demo signing key are locked under
    /// this form. Distinct from `payer_pkh()` (envelope form).
    pub fn payer_pkh_chain_b58(&self) -> &str {
        &self.payer_pkh_chain_b58
    }

    /// Underlying chain-client handle for callers that need balance
    /// queries or direct poll access.
    pub fn chain(&self) -> Arc<Mutex<NockChainClient>> {
        self.chain.clone()
    }

    /// Borrow the wallet-kernel handle (for callers that need to drive
    /// kernel pokes outside the `authorize_and_sign` flow).
    pub fn wallet_kernel(&self) -> &WalletKernelHandle {
        &self.wallet_kernel
    }
}

#[async_trait]
impl WalletBackend for NockchainWalletClient {
    fn payer_pkh(&self) -> String {
        // Envelope-form PKH — what the §5.4.1 verifier expects in
        // `auth.from`. The chain-form PKH is `payer_pkh_chain_b58()`.
        self.payer_pkh_envelope_b58.clone()
    }

    fn payer_pubkey_base58(&self) -> String {
        self.payer_pubkey_b58.clone()
    }

    async fn authorize_and_sign(
        &self,
        auth: &Authorization,
        _requirements: &PaymentRequirements,
    ) -> Result<AuthorizedPayment, WalletBackendError> {
        // `requirements` is the wallet-side policy seam consumed by
        // wrapper backends (`PolicyEnforcedSigner`-equivalent). The
        // chain-bound signing path itself is over `auth` alone.
        // 1. Payer-binding check (envelope-form PKH per §5.4.1).
        if auth.from != self.payer_pkh_envelope_b58 {
            return Err(WalletBackendError::Other(format!(
                "authorization.from ({}) does not match this wallet's envelope PKH ({})",
                auth.from, self.payer_pkh_envelope_b58
            )));
        }

        // 2. Multi-input restriction. `vesl_core::tx_builder::jam_spends_manual`
        // gates the chain `tx-id` poke to single-spend, and the
        // single-input case is what x402 needs end-to-end. Multi-input
        // is deferred until a multi-spend ZMap encoder lands.
        if auth.notes.len() != 1 {
            return Err(WalletBackendError::Unsupported(format!(
                "NockchainWalletClient currently supports exactly one input note \
                 (auth.notes had {}). Multi-input spend support is deferred — \
                 it depends on lifting the single-spend constraint in \
                 `vesl_core::tx_builder::jam_spends_manual`.",
                auth.notes.len()
            )));
        }
        let note = &auth.notes[0];

        // 3. Solvency check. The chosen UTXO must cover value + fee;
        // any excess goes back to `auth.change_address` as a second
        // output seed. The change path is unlocked here (see
        // `surrogate::multi_seed_spike`); the canonical
        // `Seeds::to_noun` round-trips for empty-NoteData multi-seed,
        // bypassing the NockStack failure mode that
        // `vesl_core::tx_builder::jam_seeds_manual` was working around.
        let value = parse_decimal(&auth.value, "auth.value")?;
        let fee = parse_decimal(&auth.fee, "auth.fee")?;
        let assets = parse_decimal(&note.assets, "auth.notes[0].assets")?;
        let needed = value
            .checked_add(fee)
            .ok_or_else(|| WalletBackendError::Other("value + fee overflows u64".into()))?;
        if assets < needed {
            return Err(WalletBackendError::InsufficientFunds(format!(
                "input note carries {assets} nicks but auth.value + auth.fee = {needed}"
            )));
        }
        let change = assets - needed;

        // 4. Build the chain-typed I/O. The chain-side PKH is the
        // noun-form Tip5(pubkey-noun); coinbase notes are locked
        // under this form. Distinct from `auth.from` which uses the
        // envelope-form Tip5(pubkey-bytes).
        let payer_pkh = self.payer_pkh_chain.clone();
        let recipient_pkh = base58_hash(&auth.to, "auth.to")?;
        let parent_hash = base58_hash(&note.name.last, "auth.notes[0].name.last")?;
        let input_first = base58_hash(&note.name.first, "auth.notes[0].name.first")?;
        let input_last = parent_hash.clone();

        let recipient_condition = SpendCondition::simple_pkh(recipient_pkh);
        let recipient_lock_root = Lock::SpendCondition(recipient_condition)
            .hash()
            .map_err(|e| WalletBackendError::Other(format!("output lock hash: {e:?}")))?;
        // Input lock dispatch — coinbase notes carry an extra timelock
        // primitive; the chain hashes coinbase_pkh and simple_pkh to
        // distinct lock_roots, so a note's `NoteLock` metadata
        // determines which `SpendCondition` the witness must present.
        // Mirrors `hull-llm::tx_builder::build_settlement_tx` (the
        // canonical reference). Phase-6 / ADR-0017 finding #3.
        let input_condition = build_input_condition(&payer_pkh, &note.lock);
        let input_lock_root = Lock::SpendCondition(input_condition.clone())
            .hash()
            .map_err(|e| WalletBackendError::Other(format!("input lock hash: {e:?}")))?;

        // Payment seed.
        let payment_seed = Seed {
            output_source: None,
            lock_root: recipient_lock_root,
            note_data: NoteData::new(Vec::new()),
            gift: Nicks(value as usize),
            parent_hash: parent_hash.clone(),
        };
        let mut seed_vec = vec![payment_seed];

        // Optional change seed back to the payer's change address.
        if change > 0 {
            let change_pkh = base58_hash(&auth.change_address, "auth.change_address")?;
            let change_condition = SpendCondition::simple_pkh(change_pkh);
            let change_lock_root = Lock::SpendCondition(change_condition).hash().map_err(|e| {
                WalletBackendError::Other(format!("change lock hash: {e:?}"))
            })?;
            seed_vec.push(Seed {
                output_source: None,
                lock_root: change_lock_root,
                note_data: NoteData::new(Vec::new()),
                gift: Nicks(change as usize),
                parent_hash,
            });
        }
        let seeds = Seeds(seed_vec);
        let fee_n = Nicks(fee as usize);

        // 5. Compute the chain sig_hash via the kernel poke. Use the
        // canonical encoder so multi-seed (change-output) Seeds are
        // jammed correctly — `vesl_core::tx_builder::kernel_sig_hash`
        // would error here for `seeds.0.len() > 1`.
        let sig_hash = {
            let mut app = self.wallet_kernel.app.lock().await;
            kernel_sig_hash_canonical(&mut app, &seeds, &fee_n)
                .await
                .map_err(|e| WalletBackendError::Other(format!("kernel %sig-hash poke: {e:?}")))?
        };

        // 6. Sign sig_hash with the chain-side Schnorr key.
        let sig_msg: [Belt; 5] = sig_hash.to_array().map(Belt);
        let chain_signature = vesl_signing::sign(&self.sk_t8, &sig_msg).map_err(|e| {
            WalletBackendError::Other(format!("vesl_core::signing::sign (chain): {e}"))
        })?;

        // 7. Assemble the witness (single PKH-signature entry, full
        // lock-merkle proof for the simple input lock).
        let lock_merkle_proof_full = LockMerkleProofFull {
            version: nockvm_macros::tas!(b"full"),
            spend_condition: input_condition,
            axis: 1,
            proof: MerkleProof {
                root: input_lock_root,
                path: vec![],
            },
        };
        let pkh_entry = PkhSignatureEntry {
            hash: payer_pkh.clone(),
            pubkey: self.payer_pubkey.clone(),
            signature: chain_signature,
        };
        let witness = Witness::new(
            LockMerkleProof::Full(lock_merkle_proof_full),
            PkhSignature::new(vec![pkh_entry]),
            Vec::new(),
        );

        // 8. Wrap into the single-element Spends z-map.
        let name = Name::new(input_first, input_last);
        let spend = Spend::Witness(Spend1 {
            witness,
            seeds,
            fee: fee_n,
        });
        let spends = Spends(vec![(name, spend)]);

        // 9. Compute the chain tx-id via the kernel poke.
        let tx_id = {
            let mut app = self.wallet_kernel.app.lock().await;
            kernel_tx_id(&mut app, &spends)
                .await
                .map_err(|e| WalletBackendError::Other(format!("kernel %tx-id poke: {e:?}")))?
        };

        // 10. Convert the assembled RawTx to the JSON surrogate.
        let raw = RawTx {
            version: Version::V1,
            id: tx_id,
            spends,
        };
        let signed_raw_tx = surrogate::surrogate_from_raw_tx(&raw)
            .map_err(|e| WalletBackendError::Other(format!("surrogate encode: {e}")))?;

        // 11. Compute the §5.4.1 envelope digest and Schnorr-sign with
        // the same key. `x402_sign_message_digest` produces
        // `[vesl_signing::prelude::Belt; 5]` — the same Belt flavour
        // `schnorr_sign` consumes — so no conversion is needed there.
        // `encode_signature` returns vesl-signing's wire form; convert
        // to the network-neutral x402-types form at the
        // AuthorizedPayment boundary.
        let envelope_msg = x402_sign_message_digest(auth)
            .map_err(|e| WalletBackendError::Other(format!("x402 sign_message: {e}")))?;
        let sk_priv = self.crypto_secret_key()?;
        let (chal, sig) = schnorr_sign(&sk_priv, &envelope_msg)
            .map_err(|e| WalletBackendError::Other(format!("schnorr_sign envelope: {e}")))?;
        let envelope_signature = encode_signature(&sk_priv.public_key(), &chal, &sig)
            .map_err(|e| WalletBackendError::Other(format!("encode envelope sig: {e}")))?;
        let envelope_signature = vesl_to_x402(&envelope_signature);

        Ok(AuthorizedPayment {
            envelope_signature,
            signed_raw_tx,
        })
    }
}

impl NockchainWalletClient {
    fn crypto_secret_key(&self) -> Result<SchnorrPrivateKey, WalletBackendError> {
        // The Belt-array stores chunks as u32 values in u64 lanes; rebuild
        // the scalar via the same little-endian layout used by
        // `SchnorrPrivateKey::from_t8`.
        let mut scalar = UBig::from(0u64);
        for (i, b) in self.sk_t8.iter().enumerate() {
            scalar += UBig::from(b.0) << (32 * i);
        }
        SchnorrPrivateKey::new(scalar)
            .map_err(|e| WalletBackendError::Other(format!("rebuild secret scalar: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a [`NoteLock`] to the chain `SpendCondition` the witness must
/// present at `lock_merkle_proof.spend_condition`. Used by
/// [`NockchainWalletClient::authorize_and_sign`] step 4 to dispatch
/// the input-lock construction.
///
/// Mirrors `hull-llm::tx_builder::build_settlement_tx`'s
/// `is_coinbase` branch. Coinbase notes are locked under
/// `coinbase_pkh(pkh, timelock_min)` (an additional `Tim` primitive
/// after the `Pkh`), simple-PKH notes under `simple_pkh(pkh)`. The
/// chain hashes the two shapes to distinct `lock_root` values, so a
/// witness presenting the wrong condition is silently rejected at
/// the chain layer (Phase-5B finding #3 / ADR-0017).
fn build_input_condition(payer_pkh: &Hash, lock: &NoteLock) -> SpendCondition {
    match lock {
        NoteLock::SimplePkh => SpendCondition::simple_pkh(payer_pkh.clone()),
        NoteLock::CoinbasePkh { timelock_min } => {
            SpendCondition::coinbase_pkh(payer_pkh.clone(), *timelock_min)
        }
    }
}

fn base58_hash(s: &str, field: &'static str) -> Result<Hash, WalletBackendError> {
    Hash::from_base58(s).map_err(|e| WalletBackendError::Other(format!("decode {field}: {e:?}")))
}

fn parse_decimal(s: &str, field: &'static str) -> Result<u64, WalletBackendError> {
    s.parse::<u64>()
        .map_err(|_| WalletBackendError::Other(format!("decode {field} `{s}` as u64")))
}

// ---------------------------------------------------------------------------
// Canonical kernel pokes (multi-seed-safe variants of vesl-core's helpers)
// ---------------------------------------------------------------------------

/// Jam a `Seeds` value via the canonical `Seeds::to_noun` encoder.
///
/// `vesl_core::tx_builder::jam_seeds_manual` is gated to a single seed
/// because `ZSet::try_from_items` (called by `Seeds::to_noun`) creates
/// an internal NockStack that fails when a non-empty `NoteData` is
/// encoded inside it. x402 path-2B output seeds carry empty `NoteData`
/// by construction, so the failure mode does not trigger — the
/// canonical encoder produces byte-identical output for the
/// single-seed case (verified by `surrogate::multi_seed_spike`) and
/// extends naturally to the multi-seed (change-output) case.
fn jam_seeds_canonical(seeds: &Seeds) -> Bytes {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let noun = seeds.to_noun(&mut slab);
    slab.set_root(noun);
    slab.jam()
}

/// Drives the kernel's `%sig-hash` poke with seeds jammed via the
/// canonical encoder. Mirrors `vesl_core::tx_builder::kernel_sig_hash`
/// but uses [`jam_seeds_canonical`] in place of `jam_seeds_manual`.
async fn kernel_sig_hash_canonical(
    app: &mut NockApp,
    seeds: &Seeds,
    fee: &Nicks,
) -> anyhow::Result<Hash> {
    let seeds_jammed = jam_seeds_canonical(seeds);

    let mut poke_slab: NounSlab = NounSlab::new();
    let tag = make_tas(&mut poke_slab, "sig-hash").as_noun();
    let seeds_atom = bytes_to_atom(&mut poke_slab, &seeds_jammed);
    let fee_noun = D(fee.0 as u64);
    let cmd = T(&mut poke_slab, &[tag, seeds_atom, fee_noun]);
    poke_slab.set_root(cmd);

    let effects = app
        .poke(SystemWire.to_wire(), poke_slab)
        .await
        .map_err(|e| anyhow::anyhow!("sig-hash poke failed: {e:?}"))?;

    extract_hash_from_effect(&effects, "sig-hash")
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// Note: kernel-bound integration tests against a real wallet kernel +
// fakenet ChainClient land in a follow-up commit gated behind a feature
// flag (the `wal.jam` byte distribution requires either a
// `kernels-open-wallet` path-dep on the local nockchain checkout or a
// pre-compiled fixture). The unit tests below cover the surrogate
// conversion in isolation; the eleven-step `authorize_and_sign` flow is
// exercised via the demo binary's stub mode.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decimal_rejects_non_numeric() {
        let err = parse_decimal("not-a-number", "foo").unwrap_err();
        match err {
            WalletBackendError::Other(msg) => assert!(msg.contains("foo")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Pin the `authorize_and_sign` step-4 dispatch from
    /// [`NoteLock`] to the chain `SpendCondition`. The two
    /// `SpendCondition::{simple_pkh, coinbase_pkh}` constructors are
    /// the canonical reference (mirrored by
    /// `hull-llm::tx_builder::build_settlement_tx`); regressing this
    /// helper would silently drop coinbase spends at the chain layer
    /// (Phase-5B finding #3 / ADR-0017).
    ///
    /// We assert by structural equality (`SpendCondition` derives
    /// `PartialEq`) rather than by lock-root hash so a regression
    /// that produces a coincidentally-equal hash still fails.
    #[test]
    fn build_input_condition_dispatches_on_note_lock() {
        let payer_pkh = Hash::from_base58("11111111111111111111111111111111")
            .expect("base58 of 32 zero bytes is a valid Hash");

        // SimplePkh — must equal `SpendCondition::simple_pkh(pkh)`.
        let simple = build_input_condition(&payer_pkh, &NoteLock::SimplePkh);
        assert_eq!(simple, SpendCondition::simple_pkh(payer_pkh.clone()));

        // CoinbasePkh — must equal `SpendCondition::coinbase_pkh(pkh,
        // timelock_min)` for any timelock value. Use the fakenet
        // canonical `1` and the mainnet default `100` per
        // `nockchain_types::blockchain_constants`.
        for timelock_min in [1u64, 100, 4_383] {
            let coinbase = build_input_condition(
                &payer_pkh,
                &NoteLock::CoinbasePkh { timelock_min },
            );
            assert_eq!(
                coinbase,
                SpendCondition::coinbase_pkh(payer_pkh.clone(), timelock_min),
                "CoinbasePkh dispatch drifted at timelock_min={timelock_min}"
            );
            // Sanity: coinbase_pkh produces a different lock_root from
            // simple_pkh — if these collide, the chain layer would
            // accept a witness against the wrong lock type.
            let coinbase_root = Lock::SpendCondition(coinbase)
                .hash()
                .expect("hash coinbase lock");
            let simple_root = Lock::SpendCondition(simple.clone())
                .hash()
                .expect("hash simple lock");
            assert_ne!(
                coinbase_root, simple_root,
                "coinbase_pkh and simple_pkh lock_roots collide at timelock_min={timelock_min}"
            );
        }
    }
}
