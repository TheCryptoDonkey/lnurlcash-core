//! Note URLs: parsing what a note claims, and building the next one.

use url::Url;

use crate::recoverable::{decode_cs1_with_amount, is_cs1_with_amount, note_id_of};
use crate::signature::{check_note, NoteCheck};
use crate::spend::{decode_note, spend_domain_of};
use crate::urls::{from_lud17, resolve_lnurl_input};

fn first_param(url: &str, key: &str) -> Option<String> {
    Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// The secret out of a note URL, normalised to lowercase hex: it is bytes, not
/// text, so casing carries no meaning, and normalising keeps duplicate
/// detection and the echo check on an informational GET from treating the same
/// secret in two casings as two different notes.
pub fn note_k1(url: &str) -> Option<String> {
    first_param(url, "k1").map(|v| v.to_ascii_lowercase())
}

/// What a note CLAIMS to carry. Only a claim by whoever encoded it - a SERVICE
/// ignores it at the informational endpoint - so it is safe to display before
/// contacting the SERVICE but must not be trusted without either a matching
/// signature or a fresh online GET. When there is no separate `amount`, a
/// current amount-bearing `cs1` carries the same declaration in its HRP.
pub fn note_declared_amount(url: &str) -> Option<u64> {
    if let Some(amount) = first_param(url, "amount") {
        return amount.parse().ok();
    }
    decode_cs1_with_amount(&note_signature(url)?).map(|cs1| cs1.amount_msat)
}

/// The certificate a note URL carries: `&c=<cs1>`. Read only: the legacy
/// `&sig=` name that LUD-25 used before 50d740a is still accepted here, so
/// notes already in circulation keep verifying; everything this crate writes
/// says `c`.
pub fn note_signature(url: &str) -> Option<String> {
    first_param(url, "c").or_else(|| first_param(url, "sig"))
}

/// Input only qualifies as a note if it resolves to a URL carrying a spend
/// that names a note: a 64-hex preimage, a `ck1` whose key is a point, or a
/// `cw1` whose control block commits to one. Anything else has no note id,
/// and would fail the first offline check later, so it is refused at the
/// door. Whether the spend opens its note is a separate question, with its
/// own answer in [`check_note_url`].
pub fn resolve_note_input(value: &str) -> Option<String> {
    let url = resolve_lnurl_input(value)?;
    let k1 = note_k1(&url)?;
    note_id_of(&k1).is_some().then_some(url)
}

pub fn is_valid_note_input(value: &str) -> bool {
    resolve_note_input(value).is_some()
}

fn rebuild(url: &Url, pairs: Vec<(String, String)>) -> String {
    let mut out = url.clone();
    out.set_query(None);
    if pairs.is_empty() {
        return out.to_string();
    }
    {
        let mut serializer = out.query_pairs_mut();
        for (key, value) in &pairs {
            serializer.append_pair(key, value);
        }
    }
    out.to_string()
}

/// A withdrawLink plus a secret makes a note. Pass `None` for `amount_msat`
/// when the real value is not known yet: the spec has a SERVICE ignore it here
/// regardless, but some implementations validate it strictly, and a placeholder
/// like 0 risks being rejected rather than ignored.
pub fn build_note_url(withdraw_link: &str, k1: &str, amount_msat: Option<u64>) -> Option<String> {
    let url = Url::parse(&from_lud17(withdraw_link.trim())).ok()?;
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "k1" && k != "amount")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    pairs.push(("k1".into(), k1.trim().to_ascii_lowercase()));
    if let Some(amount) = amount_msat {
        pairs.push(("amount".into(), amount.to_string()));
    }
    Some(rebuild(&url, pairs))
}

/// The informational GET for a note named without its spend.
///
/// LUD-25's "Checking a note without exposing it": a SERVICE MUST accept
/// `?p=<cp1>`, or a bearer note's 64-hex `h`, in place of `?k1=`, on the
/// informational GET only and never at the callback. The spend stays off the
/// wire, which is what a restore walk needs, since a walk queries a whole gap
/// window of indices the wallet has not minted into yet.
///
/// `h` is a `cp1` or a bearer note's hash, both sent as `p`: 64 hex where a
/// `cp1` goes is `h`, and a SERVICE builds the bearer note's `Q` from it.
/// [`crate::recoverable::note_lookup_of`] gives the right one for any spend.
/// A `cp1` whose key is not a point, or anything else, is `None`.
///
/// `k1`, `amount` and the certificate (`c`, or legacy `sig`) are dropped: naming the note twice, once in a form
/// that spends it, would defeat the point.
///
/// A burned note is answered exactly as one that never existed, so a
/// rejection here never distinguishes "no such note" from "spent".
pub fn build_note_info_url_by_hash(withdraw_link: &str, h: &str) -> Option<String> {
    let value = h.trim().to_ascii_lowercase();
    decode_note(&value)?;
    let url = Url::parse(&from_lud17(withdraw_link.trim())).ok()?;
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| {
            k != "k1" && k != "amount" && k != "c" && k != "sig" && k != "p" && k != "h"
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    pairs.push(("p".into(), value));
    Some(rebuild(&url, pairs))
}

/// LUD-25's offline verification of a certified note URL,
/// `lnurlw://mint.example/w?k1=<spend>&c=<cs1>`, against the `mintPubkey`
/// on record for that SERVICE: the spend must open its note at the URL's own
/// domain, and the certificate must cover that note and the amount the note
/// declares (its `amount`, or the amount a current `cs1` carries).
///
/// `None` when the URL carries no spend, no certificate or no amount, or the
/// certificate does not verify. See [`check_note`] for what comes back, and
/// [`NoteCheck::is_verified`] for the yes-or-no.
pub fn check_note_url(url: &str, mint_pubkey: &str) -> Option<NoteCheck> {
    let url = resolve_lnurl_input(url)?;
    check_note(
        &note_k1(&url)?,
        &spend_domain_of(&url)?,
        note_declared_amount(&url)?,
        &note_signature(&url)?,
        mint_pubkey,
    )
}

/// The same note with its secret swapped out, after a rotate, split or merge.
///
/// A signature only carries over when the response actually returned a fresh
/// one: a mutation with no returned signature drops any stale sig, since it no
/// longer matches the new secret.
pub fn with_new_k1(
    url: &str,
    k1: &str,
    amount_msat: u64,
    signature: Option<&str>,
) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let amount_is_implied = signature.is_some_and(is_cs1_with_amount);
    let mut pairs = Vec::new();
    let (mut saw_k1, mut saw_amount, mut saw_sig) = (false, false, false);
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "k1" => {
                pairs.push(("k1".to_string(), k1.to_ascii_lowercase()));
                saw_k1 = true;
            }
            "amount" => {
                if !amount_is_implied {
                    pairs.push(("amount".to_string(), amount_msat.to_string()));
                    saw_amount = true;
                }
            }
            // A legacy `sig` is replaced by `c`, never carried alongside it.
            "c" | "sig" => {
                if let Some(sig) = signature {
                    if !saw_sig {
                        pairs.push(("c".to_string(), sig.to_string()));
                        saw_sig = true;
                    }
                }
            }
            _ => pairs.push((key.into_owned(), value.into_owned())),
        }
    }
    if !saw_k1 {
        pairs.push(("k1".to_string(), k1.to_ascii_lowercase()));
    }
    if !saw_amount && !amount_is_implied {
        pairs.push(("amount".to_string(), amount_msat.to_string()));
    }
    if let Some(sig) = signature {
        if !saw_sig {
            pairs.push(("c".to_string(), sig.to_string()));
        }
    }
    Some(rebuild(&parsed, pairs))
}

/// Like [`with_new_k1`] but removes k1 - for re-deriving a hardware-backed
/// note's blank URL template after a mutation whose fresh secret now lives on
/// the device rather than in this process.
pub fn without_k1(url: &str, amount_msat: u64, signature: Option<&str>) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let amount_is_implied = signature.is_some_and(is_cs1_with_amount);
    let mut pairs = Vec::new();
    let (mut saw_amount, mut saw_sig) = (false, false);
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "k1" => continue,
            "amount" => {
                if !amount_is_implied {
                    pairs.push(("amount".to_string(), amount_msat.to_string()));
                    saw_amount = true;
                }
            }
            // A legacy `sig` is replaced by `c`, never carried alongside it.
            "c" | "sig" => {
                if let Some(sig) = signature {
                    if !saw_sig {
                        pairs.push(("c".to_string(), sig.to_string()));
                        saw_sig = true;
                    }
                }
            }
            _ => pairs.push((key.into_owned(), value.into_owned())),
        }
    }
    if !saw_amount && !amount_is_implied {
        pairs.push(("amount".to_string(), amount_msat.to_string()));
    }
    if let Some(sig) = signature {
        if !saw_sig {
            pairs.push(("c".to_string(), sig.to_string()));
        }
    }
    Some(rebuild(&parsed, pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn a_certificate_is_read_as_c_or_legacy_sig_and_written_as_c() {
        let current = format!("lnurlw://mint.example/w?k1={K1}&c=cs1abc");
        let legacy = format!("lnurlw://mint.example/w?k1={K1}&sig=cs1abc");
        assert_eq!(note_signature(&current).as_deref(), Some("cs1abc"));
        assert_eq!(note_signature(&legacy).as_deref(), Some("cs1abc"));

        let rewritten = with_new_k1(&legacy, K1, 21_000, Some("cs1new")).expect("rebuilds");
        assert_eq!(note_signature(&rewritten).as_deref(), Some("cs1new"));
        assert!(rewritten.contains("c=cs1new"), "{rewritten}");
        assert!(!rewritten.contains("sig="), "{rewritten}");

        let by_hash = build_note_info_url_by_hash(
            &format!("lnurlw://mint.example/w?c=cs1abc&sig=cs1abc&k1={K1}"),
            &"ab".repeat(32),
        );
        if let Some(url) = by_hash {
            assert!(!url.contains("c=cs1") && !url.contains("sig="), "{url}");
        }
    }
}
