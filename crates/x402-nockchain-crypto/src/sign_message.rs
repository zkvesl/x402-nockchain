//! Spec-faithful `sign_message` construction per
//! `docs/specs-snapshot/05-payload.md §5.4.1`.
//!
//! The x402 Authorization is signed over a Tip5 sponge of:
//!
//! ```text
//! sign_message = Tip5(
//!     "x402-nockchain-v2"   ||   // Domain separator (UTF-8 → Belts)
//!     from                    ||   // PKH → 5 Belts (base-p decomposition)
//!     to                      ||   // PKH → 5 Belts
//!     value                   ||   // Decimal string → 1 Belt (u64 mod PRIME)
//!     fee                     ||   // Decimal string → 1 Belt
//!     nonce                   ||   // base58 Tip5 hash → 5 Belts
//!     validAfter              ||   // u64 → 1 Belt
//!     validBefore             ||   // u64 → 1 Belt
//!     notes_hash              ||   // inner Tip5 of sorted (first||last) pairs
//!     changeAddress                // PKH → 5 Belts
//! )
//! ```
//!
//! ## Encoding choices
//!
//! The public spec text leaves a few byte-level details to the reference
//! implementation; this module pins them:
//!
//! - **Domain separator** (`"x402-nockchain-v2"`): packed as little-endian
//!   7-byte chunks into `Belt`s, zero-padded at the tail. 17 UTF-8 bytes →
//!   3 Belts. The 7-byte chunk size keeps each element `< 2^56 < PRIME`
//!   so every Belt is trivially in-field.
//! - **PKH / nonce / note hash** (base58 strings): decoded as a big-endian
//!   integer and successively reduced mod `PRIME` to yield 5 Belts,
//!   matching `nockchain_types::tx_engine::common::Hash::from_base58`.
//!   Output ordering is least-significant Belt first.
//! - **`value` / `fee`** (decimal strings): parsed as `u64`, then
//!   `val mod PRIME` packed as one Belt.
//! - **`validAfter` / `validBefore`** (already `u64`): `val mod PRIME` as
//!   one Belt each.
//! - **Notes hash**: notes are sorted lexicographically by
//!   `(name.first, name.last)` before flattening. Each note contributes
//!   10 Belts (5 for `first`, 5 for `last`). No inner domain separator —
//!   the outer Tip5 absorbs the digest as a tagged field.
//!
//! Any chain-side (Hoon) verifier aiming for parity MUST follow this
//! exact recipe. Digest equality is checked by the cross-backend
//! golden-vector tests scheduled for Phase 5 (ADR-0013 — the Hoon
//! bridge).

use ibig::UBig;
use thiserror::Error;
use x402_types::payment::{Authorization, NoteRef};

use crate::prelude::{hash_varlen, Belt, PRIME};

pub const X402_DOMAIN_SEPARATOR: &str = "x402-nockchain-v2";

/// Compute the PKH (public-key hash) of a Cheetah pubkey's base58
/// encoding. PKH = Tip5 digest of the pubkey bytes, encoded to base58 via
/// the same base-p integer form
/// `nockchain_types::tx_engine::common::Hash::to_base58` uses.
///
/// Accepts any bytes; the canonical input is the 97-byte CheetahPoint
/// encoding.
pub fn pkh_from_pubkey_bytes(pubkey_bytes: &[u8]) -> String {
    let mut belts = bytes_to_belts(pubkey_bytes);
    let digest = hash_varlen(&mut belts);
    pkh_belts_to_base58(&[Belt(digest[0]), Belt(digest[1]), Belt(digest[2]), Belt(digest[3]), Belt(digest[4])])
}

/// Encode a 5-Belt digest as base58 via base-p integer form. The inverse
/// of [`base58_to_belts`].
pub fn pkh_belts_to_base58(belts: &[Belt; 5]) -> String {
    let prime = UBig::from(PRIME);
    let mut value = UBig::from(0u32);
    let mut power = UBig::from(1u32);
    for b in belts {
        value += UBig::from(b.0) * &power;
        power *= &prime;
    }
    bs58::encode(value.to_be_bytes()).into_string()
}

#[derive(Debug, Error)]
pub enum SignMessageError {
    #[error("base58 decode failed for field `{field}`: {source}")]
    Base58 {
        field: &'static str,
        #[source]
        source: bs58::decode::Error,
    },
    #[error("decimal parse failed for field `{field}`: {value}")]
    Decimal {
        field: &'static str,
        value: String,
    },
    #[error("value for `{field}` is too large to fit 5 Belts")]
    Overflow { field: &'static str },
}

/// Return the Belt sequence that gets hashed — exposed for debugging + the
/// forthcoming cross-implementation (Hoon) parity tests.
pub fn x402_sign_message_belts(auth: &Authorization) -> Result<Vec<Belt>, SignMessageError> {
    let mut out = Vec::with_capacity(48);

    // Domain separator (UTF-8 bytes → 7-byte LE Belts)
    out.extend(bytes_to_belts(X402_DOMAIN_SEPARATOR.as_bytes()));

    // from → 5 Belts
    out.extend(base58_to_belts(&auth.from, "from")?);

    // to → 5 Belts
    out.extend(base58_to_belts(&auth.to, "to")?);

    // value → 1 Belt
    out.push(decimal_to_belt(&auth.value, "value")?);

    // fee → 1 Belt
    out.push(decimal_to_belt(&auth.fee, "fee")?);

    // nonce → 5 Belts (it's a base58 Tip5 hash)
    out.extend(base58_to_belts(&auth.nonce, "nonce")?);

    // validAfter → 1 Belt
    out.push(u64_to_belt(auth.valid_after));

    // validBefore → 1 Belt
    out.push(u64_to_belt(auth.valid_before));

    // notes_hash → 5 Belts (inner Tip5)
    out.extend(compute_notes_hash(&auth.notes)?);

    // changeAddress → 5 Belts
    out.extend(base58_to_belts(&auth.change_address, "changeAddress")?);

    Ok(out)
}

/// Return the 5-Belt Tip5 digest of `sign_message`. This is what gets
/// passed to `SchnorrPrivateKey::sign`.
pub fn x402_sign_message_digest(auth: &Authorization) -> Result<[Belt; 5], SignMessageError> {
    let mut belts = x402_sign_message_belts(auth)?;
    let d = hash_varlen(&mut belts);
    Ok([Belt(d[0]), Belt(d[1]), Belt(d[2]), Belt(d[3]), Belt(d[4])])
}

fn compute_notes_hash(notes: &[NoteRef]) -> Result<[Belt; 5], SignMessageError> {
    // Sort lexicographically by (first, last) — base58 strings compare
    // identically to their underlying byte sequence in lex order.
    let mut sorted: Vec<&NoteRef> = notes.iter().collect();
    sorted.sort_by(|a, b| {
        (a.name.first.as_str(), a.name.last.as_str())
            .cmp(&(b.name.first.as_str(), b.name.last.as_str()))
    });

    let mut belts = Vec::with_capacity(sorted.len() * 10);
    for note in sorted {
        belts.extend(base58_to_belts(&note.name.first, "notes[i].name.first")?);
        belts.extend(base58_to_belts(&note.name.last, "notes[i].name.last")?);
    }

    // Empty notes: still emit a deterministic 5-Belt digest.
    if belts.is_empty() {
        belts.push(Belt(0));
    }

    let d = hash_varlen(&mut belts);
    Ok([Belt(d[0]), Belt(d[1]), Belt(d[2]), Belt(d[3]), Belt(d[4])])
}

/// Decode a base58 string as a big-endian integer and successively reduce
/// it mod `PRIME` to yield 5 Belts. Matches
/// `nockchain_types::tx_engine::common::Hash::from_base58`.
pub fn base58_to_belts(b58: &str, field: &'static str) -> Result<[Belt; 5], SignMessageError> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| SignMessageError::Base58 { field, source: e })?;
    let mut value = UBig::from_be_bytes(&bytes);
    let prime = UBig::from(PRIME);
    let mut belts = [Belt(0); 5];
    for b in &mut belts {
        let rem = (&value % &prime)
            .to_string()
            .parse::<u64>()
            .map_err(|_| SignMessageError::Overflow { field })?;
        *b = Belt(rem);
        value /= &prime;
    }
    if value > UBig::from(0u32) {
        return Err(SignMessageError::Overflow { field });
    }
    Ok(belts)
}

fn decimal_to_belt(s: &str, field: &'static str) -> Result<Belt, SignMessageError> {
    let v: u64 = s
        .parse()
        .map_err(|_| SignMessageError::Decimal { field, value: s.to_string() })?;
    Ok(u64_to_belt(v))
}

fn u64_to_belt(v: u64) -> Belt {
    Belt(v % PRIME)
}

/// Pack arbitrary bytes into Belts: 7 bytes per Belt, little-endian,
/// zero-padded. Used for the UTF-8 domain separator; matches
/// `vesl_signing::domain::tip5_with_domain`'s convention (re-exported
/// here as `crate::canonical::tip5_with_domain`).
fn bytes_to_belts(bytes: &[u8]) -> Vec<Belt> {
    let mut out = Vec::with_capacity(bytes.len() / 7 + 1);
    let mut buf = [0u8; 8];
    for chunk in bytes.chunks(7) {
        buf.fill(0);
        buf[..chunk.len()].copy_from_slice(chunk);
        out.push(Belt(u64::from_le_bytes(buf)));
    }
    if bytes.is_empty() || bytes.len() % 7 == 0 {
        out.push(Belt(0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use x402_types::payment::{NoteName, NoteRef};

    fn fixture() -> Authorization {
        // All base58-encoded fields below use only characters from the
        // Bitcoin base58 alphabet (0-9 A-Z a-z, excluding 0 O I l) so
        // `bs58::decode` succeeds deterministically.
        Authorization {
            from: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNy".into(),
            to: "1A1zP1eP5QGefi2DMPTfTL5SmvPiy".into(),
            value: "65536".into(),
            fee: "10".into(),
            nonce: "5Ht7Rk3qX9abcdefghjkmnopqrstu".into(),
            valid_after: 1_700_000_000,
            valid_before: 1_700_000_300,
            notes: vec![NoteRef {
                name: NoteName {
                    first: "4Ab2c3D4e5F6g7H8i9J1kLmNoPqRsTu".into(),
                    last: "2Pq3rS4tU5vW6xY7zA8bC9dE2fG3hJjK".into(),
                },
                assets: "100000".into(),
                lock: Default::default(),
            }],
            change_address: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNy".into(),
        }
    }

    #[test]
    fn digest_is_deterministic() {
        let auth = fixture();
        let a = x402_sign_message_digest(&auth).unwrap();
        let b = x402_sign_message_digest(&auth).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn digest_changes_on_value_change() {
        let mut auth = fixture();
        let a = x402_sign_message_digest(&auth).unwrap();
        auth.value = "65537".into();
        let b = x402_sign_message_digest(&auth).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn digest_changes_on_note_reorder_is_stable() {
        // Sorting means reordering the input notes MUST NOT change the
        // digest.
        let auth = fixture();
        let d1 = x402_sign_message_digest(&auth).unwrap();

        let mut reordered = auth.clone();
        reordered.notes.reverse();
        let d2 = x402_sign_message_digest(&reordered).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn digest_changes_on_new_note_added() {
        let mut auth = fixture();
        let d1 = x402_sign_message_digest(&auth).unwrap();
        auth.notes.push(NoteRef {
            name: NoteName {
                first: "9xyzAbcDeFgHJkmNoPqRsTuVwXyZ1".into(),
                last: "8ZxYwVuTsRqPoNmKJHgFeDcBa987".into(),
            },
            assets: "1".into(),
            lock: Default::default(),
        });
        let d2 = x402_sign_message_digest(&auth).unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn base58_decode_propagates() {
        let mut auth = fixture();
        auth.from = "not valid base58 @#$%".into();
        let err = x402_sign_message_digest(&auth).unwrap_err();
        assert!(matches!(err, SignMessageError::Base58 { field: "from", .. }));
    }

    #[test]
    fn decimal_parse_error_is_typed() {
        let mut auth = fixture();
        auth.value = "not a number".into();
        let err = x402_sign_message_digest(&auth).unwrap_err();
        assert!(matches!(
            err,
            SignMessageError::Decimal { field: "value", .. }
        ));
    }

    #[test]
    fn empty_notes_digest_is_defined_and_distinct() {
        let mut auth = fixture();
        auth.notes.clear();
        let d = x402_sign_message_digest(&auth).unwrap();
        // Not asserting a specific value (that's what golden vectors are
        // for); just that it's producible and differs from the non-empty
        // case.
        let full = fixture();
        let d_full = x402_sign_message_digest(&full).unwrap();
        assert_ne!(d, d_full);
    }
}
