//! `SignedRawTx` ↔ `RawTx` bridge.
//!
//! Owns both directions of the JSON-surrogate ↔ noun-typed conversion
//! defined by `docs/wallet-integration.md §7`. The surrogate's hybrid
//! shape — typed top-level fields, base64'd jammed nouns for sub-trees
//! that lack natural JSON forms — means every conversion site touches:
//!
//! - **Hash → base58 / base58 → Hash** for identity-shaped fields
//!   (`tx_id`, `lock_root`, `parent_hash`, PKH).
//! - **`SchnorrSignature` → `[String; 8]` × 2** for the Schnorr
//!   `(chal, sig)` pair, kept typed because every payment auditor wants
//!   to read who signed.
//! - **Jam + base64** for `LockMerkleProof`, `Option<Source>`, `NoteData`,
//!   `HaxPreimage::value` — opaque by construction, decoded back via
//!   `cue` at the verifier.
//!
//! Both consumers (the wallet client when assembling a payment, and the
//! facilitator when submitting one) reach for the same module so the
//! wire shape stays exactly one source of truth.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use bytes::Bytes;
use nockapp::noun::slab::{NockJammer, NounSlab};
use nockchain_math::belt::Belt;
use nockchain_types::tx_engine::common::{
    Hash, Name, Nicks, SchnorrPubkey, SchnorrSignature, Source, Version,
};
use nockchain_types::tx_engine::v1::note::NoteData;
use nockchain_types::tx_engine::v1::tx::{
    HaxPreimage, LockMerkleProof, PkhSignature, PkhSignatureEntry, Seed, Seeds, Spend, Spend1,
    Spends, Witness,
};
use nockchain_types::tx_engine::v1::RawTx;
use noun_serde::{NounDecode, NounEncode};
use x402_types::nockchain::{
    SignedHaxPreimage, SignedNoteName, SignedPkhSignatureEntry, SignedRawTx, SignedSchnorrAtoms,
    SignedSeed, SignedSpend, SignedSpendEntry, SignedWitness,
};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

// ---------------------------------------------------------------------------
// RawTx → SignedRawTx (forward)
// ---------------------------------------------------------------------------

/// Encode a chain-typed `RawTx` as the JSON surrogate the wallet ships
/// inside the x402 payload.
///
/// Returns an error if the tx version isn't `V1` or if any nested noun
/// fails to encode (the latter would indicate a bug — `to_noun` is
/// infallible for the types involved here).
pub fn surrogate_from_raw_tx(raw: &RawTx) -> Result<SignedRawTx> {
    if raw.version != Version::V1 {
        bail!("only v1 RawTx is supported by the SignedRawTx surrogate");
    }
    let mut spends = Vec::with_capacity(raw.spends.0.len());
    for (name, spend) in raw.spends.0.iter() {
        spends.push(signed_spend_entry(name, spend)?);
    }
    Ok(SignedRawTx {
        version: "v1".into(),
        tx_id: raw.id.to_base58(),
        spends,
    })
}

fn signed_spend_entry(name: &Name, spend: &Spend) -> Result<SignedSpendEntry> {
    let spend1 = match spend {
        Spend::Witness(s) => s,
        Spend::Legacy(_) => bail!("legacy (v0) spends are not supported by the surrogate"),
    };
    Ok(SignedSpendEntry {
        name: SignedNoteName {
            first: name.first.to_base58(),
            last: name.last.to_base58(),
        },
        spend: signed_spend(spend1)?,
    })
}

fn signed_spend(spend: &Spend1) -> Result<SignedSpend> {
    Ok(SignedSpend {
        witness: signed_witness(&spend.witness)?,
        seeds: spend
            .seeds
            .0
            .iter()
            .map(signed_seed)
            .collect::<Result<Vec<_>>>()?,
        fee: spend.fee.0.to_string(),
    })
}

fn signed_witness(witness: &Witness) -> Result<SignedWitness> {
    let lock_merkle_proof_noun = encode_noun(&witness.lock_merkle_proof);
    let pkh_signature = witness
        .pkh_signature
        .0
        .iter()
        .map(signed_pkh_entry)
        .collect::<Result<Vec<_>>>()?;
    let hax = witness
        .hax
        .iter()
        .map(signed_hax)
        .collect::<Result<Vec<_>>>()?;
    Ok(SignedWitness {
        lock_merkle_proof_noun,
        pkh_signature,
        hax,
        tim: witness.tim as u64,
    })
}

fn signed_pkh_entry(entry: &PkhSignatureEntry) -> Result<SignedPkhSignatureEntry> {
    Ok(SignedPkhSignatureEntry {
        hash: entry.hash.to_base58(),
        pubkey: entry
            .pubkey
            .to_base58()
            .map_err(|e| anyhow!("encode pubkey to base58: {e:?}"))?,
        signature: schnorr_to_atoms(&entry.signature),
    })
}

fn signed_hax(hax: &HaxPreimage) -> Result<SignedHaxPreimage> {
    // `HaxPreimage::value` is already a jammed-noun byte string per the
    // upstream type's contract — just base64 it through.
    Ok(SignedHaxPreimage {
        hash: hax.hash.to_base58(),
        value_noun: B64.encode(&hax.value),
    })
}

fn signed_seed(seed: &Seed) -> Result<SignedSeed> {
    let output_source_noun = seed.output_source.as_ref().map(|src| encode_noun(src));
    let note_data_noun = if seed.note_data.0.is_empty() {
        None
    } else {
        Some(encode_noun(&seed.note_data))
    };
    Ok(SignedSeed {
        output_source_noun,
        lock_root: seed.lock_root.to_base58(),
        note_data_noun,
        gift: seed.gift.0.to_string(),
        parent_hash: seed.parent_hash.to_base58(),
    })
}

fn schnorr_to_atoms(sig: &SchnorrSignature) -> SignedSchnorrAtoms {
    SignedSchnorrAtoms {
        chal: std::array::from_fn(|i| sig.chal[i].0.to_string()),
        sig: std::array::from_fn(|i| sig.sig[i].0.to_string()),
    }
}

fn encode_noun<T: NounEncode>(value: &T) -> String {
    let mut slab: NounSlab<NockJammer> = NounSlab::new();
    let noun = value.to_noun(&mut slab);
    slab.set_root(noun);
    B64.encode(slab.jam())
}

// ---------------------------------------------------------------------------
// SignedRawTx → RawTx (inverse)
// ---------------------------------------------------------------------------

/// Decode a JSON surrogate back to a chain-typed `RawTx` ready for
/// `nockchain_client_rs::ChainClient::submit_and_wait`.
pub fn raw_tx_from_surrogate(signed: &SignedRawTx) -> Result<RawTx> {
    if signed.version != "v1" {
        bail!(
            "unsupported SignedRawTx version `{}` (only `v1` is accepted)",
            signed.version
        );
    }
    let id = Hash::from_base58(&signed.tx_id)
        .map_err(|e| anyhow!("decode SignedRawTx.tx_id: {e:?}"))?;
    let mut spends = Vec::with_capacity(signed.spends.len());
    for entry in signed.spends.iter() {
        spends.push(spend_pair_from_surrogate(entry)?);
    }
    Ok(RawTx {
        version: Version::V1,
        id,
        spends: Spends(spends),
    })
}

fn spend_pair_from_surrogate(entry: &SignedSpendEntry) -> Result<(Name, Spend)> {
    let first = Hash::from_base58(&entry.name.first)
        .map_err(|e| anyhow!("decode spend name.first: {e:?}"))?;
    let last = Hash::from_base58(&entry.name.last)
        .map_err(|e| anyhow!("decode spend name.last: {e:?}"))?;
    let name = Name::new(first, last);
    let spend = Spend::Witness(spend1_from_surrogate(&entry.spend)?);
    Ok((name, spend))
}

fn spend1_from_surrogate(s: &SignedSpend) -> Result<Spend1> {
    let witness = witness_from_surrogate(&s.witness)?;
    let mut seeds = Vec::with_capacity(s.seeds.len());
    for seed in s.seeds.iter() {
        seeds.push(seed_from_surrogate(seed)?);
    }
    let fee_u64: u64 = s
        .fee
        .parse()
        .map_err(|_| anyhow!("decode SignedSpend.fee `{}` as u64", s.fee))?;
    Ok(Spend1 {
        witness,
        seeds: Seeds(seeds),
        fee: Nicks(fee_u64 as usize),
    })
}

fn witness_from_surrogate(w: &SignedWitness) -> Result<Witness> {
    let lock_merkle_proof: LockMerkleProof = decode_noun(&w.lock_merkle_proof_noun)
        .context("decode SignedWitness.lock_merkle_proof_noun")?;
    let mut pkh_entries = Vec::with_capacity(w.pkh_signature.len());
    for entry in w.pkh_signature.iter() {
        pkh_entries.push(pkh_entry_from_surrogate(entry)?);
    }
    let mut hax = Vec::with_capacity(w.hax.len());
    for entry in w.hax.iter() {
        hax.push(hax_from_surrogate(entry)?);
    }
    Ok(Witness {
        lock_merkle_proof,
        pkh_signature: PkhSignature::new(pkh_entries),
        hax,
        tim: w.tim as usize,
    })
}

fn pkh_entry_from_surrogate(e: &SignedPkhSignatureEntry) -> Result<PkhSignatureEntry> {
    let hash = Hash::from_base58(&e.hash)
        .map_err(|err| anyhow!("decode pkh entry hash: {err:?}"))?;
    let pubkey = SchnorrPubkey::from_base58(&e.pubkey)
        .map_err(|err| anyhow!("decode pkh entry pubkey: {err:?}"))?;
    let signature = schnorr_from_atoms(&e.signature)?;
    Ok(PkhSignatureEntry {
        hash,
        pubkey,
        signature,
    })
}

fn hax_from_surrogate(h: &SignedHaxPreimage) -> Result<HaxPreimage> {
    let hash = Hash::from_base58(&h.hash)
        .map_err(|e| anyhow!("decode HaxPreimage.hash: {e:?}"))?;
    // Preserve raw value bytes (jammed-noun atom encoded as base64).
    let value_bytes = B64
        .decode(h.value_noun.as_bytes())
        .context("decode HaxPreimage.value_noun base64")?;
    Ok(HaxPreimage {
        hash,
        value: Bytes::from(value_bytes),
    })
}

fn seed_from_surrogate(s: &SignedSeed) -> Result<Seed> {
    let output_source = match &s.output_source_noun {
        Some(b64) => Some(decode_noun::<Source>(b64).context("decode Seed.output_source")?),
        None => None,
    };
    let lock_root = Hash::from_base58(&s.lock_root)
        .map_err(|e| anyhow!("decode Seed.lock_root: {e:?}"))?;
    let note_data = match &s.note_data_noun {
        Some(b64) => decode_noun::<NoteData>(b64).context("decode Seed.note_data")?,
        None => NoteData::new(Vec::new()),
    };
    let gift_u64: u64 = s
        .gift
        .parse()
        .map_err(|_| anyhow!("decode Seed.gift `{}` as u64", s.gift))?;
    let parent_hash = Hash::from_base58(&s.parent_hash)
        .map_err(|e| anyhow!("decode Seed.parent_hash: {e:?}"))?;
    Ok(Seed {
        output_source,
        lock_root,
        note_data,
        gift: Nicks(gift_u64 as usize),
        parent_hash,
    })
}

fn schnorr_from_atoms(atoms: &SignedSchnorrAtoms) -> Result<SchnorrSignature> {
    let chal = parse_belt_array(&atoms.chal, "chal")?;
    let sig = parse_belt_array(&atoms.sig, "sig")?;
    Ok(SchnorrSignature { chal, sig })
}

fn parse_belt_array(arr: &[String; 8], field: &'static str) -> Result<[Belt; 8]> {
    let mut out = [Belt(0); 8];
    for (i, s) in arr.iter().enumerate() {
        let v: u64 = s
            .parse()
            .map_err(|_| anyhow!("decode {field}[{i}] `{s}` as u64"))?;
        out[i] = Belt(v);
    }
    Ok(out)
}

fn decode_noun<T: NounDecode>(b64: &str) -> Result<T> {
    let bytes = B64
        .decode(b64.as_bytes())
        .context("decode jammed-noun base58")?;
    let mut slab: NounSlab = NounSlab::new();
    let cued = slab
        .cue_into(Bytes::from(bytes))
        .map_err(|e| anyhow!("cue jammed bytes: {e:?}"))?;
    T::from_noun(&cued).map_err(|e| anyhow!("decode noun: {e:?}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nockchain_types::tx_engine::common::Hash;
    use nockchain_types::tx_engine::v1::tx::{
        Lock, LockMerkleProofFull, MerkleProof, SpendCondition,
    };

    fn fake_hash(seed: u64) -> Hash {
        Hash::from_limbs(&[seed, seed + 1, seed + 2, seed + 3, seed + 4])
    }

    fn fake_pubkey() -> SchnorrPubkey {
        // sk = 2 → derive a real public key on the Cheetah curve.
        let sk: [Belt; 8] = std::array::from_fn(|i| if i == 0 { Belt(2) } else { Belt(0) });
        vesl_core::signing::derive_pubkey(&sk)
    }

    fn fake_signature() -> SchnorrSignature {
        SchnorrSignature {
            chal: std::array::from_fn(|i| Belt((i + 1) as u64)),
            sig: std::array::from_fn(|i| Belt((i + 11) as u64)),
        }
    }

    fn fake_raw_tx() -> RawTx {
        let payer_pkh = fake_hash(7);
        let recipient_pkh = fake_hash(17);
        let input_condition = SpendCondition::simple_pkh(payer_pkh.clone());
        let input_lock_root = Lock::SpendCondition(input_condition.clone())
            .hash()
            .expect("lock hash");
        let recipient_condition = SpendCondition::simple_pkh(recipient_pkh.clone());
        let recipient_lock_root = Lock::SpendCondition(recipient_condition)
            .hash()
            .expect("lock hash");

        let lock_merkle_proof = LockMerkleProof::Full(LockMerkleProofFull {
            version: nockvm_macros::tas!(b"full"),
            spend_condition: input_condition,
            axis: 1,
            proof: MerkleProof {
                root: input_lock_root,
                path: vec![],
            },
        });

        let pkh_entry = PkhSignatureEntry {
            hash: payer_pkh,
            pubkey: fake_pubkey(),
            signature: fake_signature(),
        };

        let witness = Witness::new(lock_merkle_proof, PkhSignature::new(vec![pkh_entry]), vec![]);

        let seed = Seed {
            output_source: None,
            lock_root: recipient_lock_root,
            note_data: NoteData::new(vec![]),
            gift: Nicks(990),
            parent_hash: fake_hash(101),
        };

        let spend = Spend::Witness(Spend1 {
            witness,
            seeds: Seeds(vec![seed]),
            fee: Nicks(10),
        });

        let name = Name::new(fake_hash(201), fake_hash(202));
        RawTx {
            version: Version::V1,
            id: fake_hash(999),
            spends: Spends(vec![(name, spend)]),
        }
    }

    #[test]
    fn raw_tx_round_trips_through_surrogate() {
        let original = fake_raw_tx();
        let surrogate = surrogate_from_raw_tx(&original).expect("forward conversion");

        // Top-level fields decode back to the same chain-typed values.
        let recovered = raw_tx_from_surrogate(&surrogate).expect("inverse conversion");

        assert_eq!(recovered.version, original.version);
        assert_eq!(recovered.id, original.id);
        assert_eq!(recovered.spends.0.len(), original.spends.0.len());

        let (orig_name, orig_spend) = &original.spends.0[0];
        let (rec_name, rec_spend) = &recovered.spends.0[0];
        assert_eq!(rec_name, orig_name);

        let orig_spend1 = match orig_spend {
            Spend::Witness(s) => s,
            _ => panic!("expected witness spend"),
        };
        let rec_spend1 = match rec_spend {
            Spend::Witness(s) => s,
            _ => panic!("expected witness spend"),
        };

        assert_eq!(rec_spend1.fee, orig_spend1.fee);
        assert_eq!(rec_spend1.seeds.0.len(), orig_spend1.seeds.0.len());
        assert_eq!(rec_spend1.witness.tim, orig_spend1.witness.tim);
        assert_eq!(
            rec_spend1.witness.pkh_signature.0.len(),
            orig_spend1.witness.pkh_signature.0.len()
        );

        let orig_pkh_entry = &orig_spend1.witness.pkh_signature.0[0];
        let rec_pkh_entry = &rec_spend1.witness.pkh_signature.0[0];
        assert_eq!(rec_pkh_entry.hash, orig_pkh_entry.hash);
        assert_eq!(rec_pkh_entry.pubkey, orig_pkh_entry.pubkey);
        assert_eq!(rec_pkh_entry.signature, orig_pkh_entry.signature);

        let orig_seed = &orig_spend1.seeds.0[0];
        let rec_seed = &rec_spend1.seeds.0[0];
        assert_eq!(rec_seed.lock_root, orig_seed.lock_root);
        assert_eq!(rec_seed.gift, orig_seed.gift);
        assert_eq!(rec_seed.parent_hash, orig_seed.parent_hash);
        assert!(rec_seed.note_data.0.is_empty());
    }

    #[test]
    fn surrogate_emits_typed_payment_metadata() {
        let raw = fake_raw_tx();
        let s = surrogate_from_raw_tx(&raw).expect("forward conversion");
        assert_eq!(s.version, "v1");
        assert_eq!(s.spends[0].spend.fee, "10");
        assert_eq!(s.spends[0].spend.seeds[0].gift, "990");
        // PKH signature is base58-typed, not opaque.
        assert!(!s.spends[0].spend.witness.pkh_signature[0].hash.is_empty());
        // Lock merkle proof is opaque (base64 of jammed noun).
        assert!(!s.spends[0]
            .spend
            .witness
            .lock_merkle_proof_noun
            .is_empty());
    }

    #[test]
    fn rejects_unknown_version() {
        let mut s = surrogate_from_raw_tx(&fake_raw_tx()).unwrap();
        s.version = "v0".into();
        assert!(raw_tx_from_surrogate(&s).is_err());
    }
}

// ---------------------------------------------------------------------------
// Regression guard: the canonical `Seeds::to_noun` encoder is what
// `lib.rs::jam_seeds_canonical` and `lib.rs::kernel_sig_hash_canonical`
// rely on to support multi-seed (change-output) spends.
//
// Background: `vesl_core::tx_builder::jam_seeds_manual` exists as a
// workaround for `NoteData::to_noun()` failing inside the NockStack
// created by `ZSet::try_from_items`. x402 path-2B output seeds carry
// empty `NoteData` by construction, so the failure mode does not
// trigger here — confirmed by both tests below. If either regresses,
// the multi-output path in `authorize_and_sign` must be reverted to
// the single-seed-only restriction until an upstream fix lands.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod multi_seed_spike {
    use super::*;
    use nockapp::noun::slab::{NockJammer, NounSlab};
    use nockchain_types::tx_engine::common::Hash;

    fn empty_seed(lock: u64, parent: u64, gift: usize) -> Seed {
        Seed {
            output_source: None,
            lock_root: Hash::from_limbs(&[lock, lock + 1, lock + 2, lock + 3, lock + 4]),
            note_data: NoteData::new(vec![]),
            gift: Nicks(gift),
            parent_hash: Hash::from_limbs(&[parent, parent, parent, parent, parent]),
        }
    }

    /// Multi-seed (2-seed) Seeds with empty NoteData encodes via the
    /// canonical `Seeds::to_noun` path without panicking, jams to bytes,
    /// cues back, and round-trips under `Seeds::from_noun`.
    ///
    /// If this test passes, the NockStack failure mode behind the
    /// `jam_seeds_manual` workaround is non-load-bearing for x402 — and
    /// the single-seed restriction in `authorize_and_sign` can be lifted
    /// for the change-output case.
    #[test]
    fn two_empty_note_data_seeds_round_trip_via_canonical_encoder() {
        let seeds = Seeds(vec![
            empty_seed(1, 100, 500),
            empty_seed(2, 100, 490),
        ]);

        // Forward: canonical Seeds::to_noun -> jam.
        let mut slab: NounSlab<NockJammer> = NounSlab::new();
        let noun = seeds.to_noun(&mut slab);
        slab.set_root(noun);
        let jammed = slab.jam();
        assert!(!jammed.is_empty(), "jammed bytes must be non-empty");

        // Inverse: cue -> Seeds::from_noun.
        let mut decode_slab: NounSlab = NounSlab::new();
        let cued = decode_slab
            .cue_into(jammed)
            .expect("cue should succeed for empty-NoteData multi-seed");
        let decoded = Seeds::from_noun(&cued).expect("decode multi-seed Seeds");

        assert_eq!(decoded.0.len(), 2);
        // Order of items in the z-set is determined by the underlying
        // treap layout, not insertion order — assert membership rather
        // than positional equality.
        let mut decoded_lock_roots: Vec<_> =
            decoded.0.iter().map(|s| s.lock_root.clone()).collect();
        decoded_lock_roots.sort_by_key(|h| h.to_array());
        let mut original_lock_roots: Vec<_> =
            seeds.0.iter().map(|s| s.lock_root.clone()).collect();
        original_lock_roots.sort_by_key(|h| h.to_array());
        assert_eq!(decoded_lock_roots, original_lock_roots);
    }

    /// Sanity check: with **one** empty-NoteData seed, the canonical
    /// encoder produces the same jammed bytes as
    /// `vesl_core::tx_builder::jam_seeds_manual`. If this passes, the
    /// canonical path is interchangeable with the manual helper for the
    /// single-seed case too — no need to keep the manual helper as a
    /// separate code path.
    #[test]
    fn single_seed_canonical_matches_manual_jam() {
        let seeds = Seeds(vec![empty_seed(1, 100, 990)]);

        let manual = vesl_core::tx_builder::jam_seeds_manual(&seeds)
            .expect("manual jam should succeed");

        let canonical = {
            let mut slab: NounSlab<NockJammer> = NounSlab::new();
            let noun = seeds.to_noun(&mut slab);
            slab.set_root(noun);
            slab.jam()
        };

        assert_eq!(
            manual.to_vec(),
            canonical.to_vec(),
            "canonical Seeds::to_noun jam must match jam_seeds_manual for a single empty-NoteData seed"
        );
    }
}
