//! Permissive deserializers for the numeric and identifier fields of the OpenSubsonic
//! payloads.
//!
//! **Why this module exists.** `serde` is strict about *types*, not just about absence.
//! Given `#[derive(Deserialize)]` on `year: Option<u32>`, a server that sends
//! `"year": "1998"` — a JSON *string* — does not yield `None`; it fails the field, which
//! fails the record, which (before [`crate::parse`] started skipping bad records) failed
//! the whole page. The old TypeScript client had no such failure mode: every payload
//! crossed an unchecked `as SubSong[]` cast and the value was passed through untouched.
//!
//! That difference was not theoretical, and its symptom was much worse than a blank grid.
//! `useSubsonic.ts`'s `connect()` fetches the first album page **inside** its try block,
//! so one string-encoded `year` on page 1 threw before the connect completed;
//! `friendlyConnectError` matched neither its transport patterns nor its 401 patterns and
//! fell through to `return raw`, leaving the user staring at a connect panel that read
//! literally **"Bad response"** against a server the previous release handled fine.
//! Navidrome is well-behaved here. Gonic, Ampache, Astiga and LMS are looser, and this
//! crate ships to a public MIT repo where people point it at exactly those.
//!
//! **The rule these helpers implement**, chosen to sit as close to the old TypeScript as
//! a typed field can:
//!
//! * A JSON number is taken as-is (floats truncate toward zero — `251.5` → `251`).
//! * A numeric *string* is parsed (`"1998"` → `1998`, `"-7.2"` → `-7.2`).
//! * Anything that cannot sensibly become the target type — a negative year, a bool, an
//!   object, `"unknown"`, `NaN`, a value past the type's range — is treated as
//!   **absent** (`None`), never as an error. A display field EKO cannot read is a field
//!   it renders as blank, exactly like a server that omitted it.
//!
//! [`id`] is the deliberate exception: it is load-bearing (streaming, downloading,
//! scrobbling and art all key off it, and [`crate::urls::attach_song_urls`] would mint
//! four garbage URLs from an unusable one), so it accepts a string *or* a number — some
//! servers use integer ids — and **errors** on anything else, **including the empty
//! string**. Routing every `id` in the crate through one helper is what finally makes that
//! true: the plain `String` derive it replaced accepted `""`, which is precisely the case
//! that mints four URLs ending `&id=`. That error is what [`crate::parse`]'s
//! element-by-element collection reader turns into "skip this record", which is why a
//! record with an unusable id disappears from a page instead of taking the page down with
//! it — and [`vec_skipping_bad`] says so on stderr, so a short page is diagnosable rather
//! than silent.
//!
//! Nothing here touches `Serialize`: the fields keep their `Option<u32>` / `String`
//! types, so the JSON that crosses the Tauri IPC boundary to the frontend is byte-for-byte
//! what it was before.

use serde::de::{DeserializeOwned, Deserializer, Error as _, Unexpected};
use serde::Deserialize;
use serde_json::Value;

// ── Value → scalar (the shared core, also called directly by `parse::genres`) ──

/// A JSON value read as a `u32`, or `None` if it cannot sensibly be one.
///
/// Accepts numbers and numeric strings. Negatives, non-finite floats, values past
/// `u32::MAX`, and anything non-numeric are `None`. Floats truncate toward zero.
pub fn u32_from_value(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                return u32::try_from(u).ok();
            }
            if n.as_i64().is_some() {
                return None; // negative: `"year": -1` is "unknown", not 4294967295
            }
            n.as_f64().and_then(f64_to_u32)
        }
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return None;
            }
            t.parse::<u32>()
                .ok()
                .or_else(|| t.parse::<f64>().ok().and_then(f64_to_u32))
        }
        _ => None,
    }
}

fn f64_to_u32(f: f64) -> Option<u32> {
    if f.is_finite() && f >= 0.0 && f <= f64::from(u32::MAX) {
        Some(f as u32)
    } else {
        None
    }
}

/// A JSON value read as an `f64`, or `None` if it cannot sensibly be one.
///
/// Accepts numbers and numeric strings; `NaN`/infinities are `None`. This is the
/// ReplayGain path: `{"trackGain": "-7.2"}` is a real shape in the wild.
pub fn f64_from_value(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

// ── serde entry points (`#[serde(deserialize_with = …)]`) ─────────────────────

/// `Option<u32>` that accepts a number or a numeric string and degrades to `None`.
///
/// Always pair with `#[serde(default)]` — `deserialize_with` does not imply it, and an
/// absent key must stay absent rather than becoming an error.
pub fn opt_u32<'de, D>(d: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(u32_from_value(&Value::deserialize(d)?))
}

/// `Option<f64>` that accepts a number or a numeric string and degrades to `None`.
pub fn opt_f64<'de, D>(d: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(f64_from_value(&Value::deserialize(d)?))
}

/// An `id`: a JSON string, or a JSON number rendered as its decimal string.
///
/// Unlike the helpers above this one **fails** on anything else, because there is no
/// useful "absent id" — see the module docs.
///
/// **An empty (or whitespace-only) string is rejected too**, and that is the point of
/// routing every `id` in the crate through here. The plain `String` derive this replaced
/// accepted `""` happily, which was the one case that actually did what three doc comments
/// warned about: [`crate::urls::attach_song_urls`] would mint four URLs ending `&id=` and
/// the frontend would render a track that could never stream, download or scrobble.
/// "Missing" and "present but empty" are the same thing to every consumer, so they are the
/// same thing here.
pub fn id<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(d)? {
        Value::String(s) if s.trim().is_empty() => Err(D::Error::invalid_value(
            Unexpected::Str(&s),
            &"a non-empty id",
        )),
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Err(D::Error::invalid_type(Unexpected::Bool(b), &"an id")),
        Value::Null => Err(D::Error::invalid_type(Unexpected::Unit, &"an id")),
        Value::Array(_) => Err(D::Error::invalid_type(Unexpected::Seq, &"an id")),
        Value::Object(_) => Err(D::Error::invalid_type(Unexpected::Map, &"an id")),
    }
}

// ── list helpers ──────────────────────────────────────────────────────────────

/// Deserialize a JSON array element-by-element, **dropping** elements that fail.
///
/// This is the array-level half of the same philosophy: one malformed record must not take
/// down the page around it.
///
/// **A bare object is tried as a single element.** Several XML-derived Subsonic JSON
/// encoders collapse a one-element list into the object itself (`"album": {…}` rather than
/// `"album": [{…}]`), and that shape is the highest-ranked remaining non-Navidrome risk in
/// the QA checklist. Reading it as one record cannot be worse than the alternative: before
/// this arm existed it fell to the catch-all and produced an **empty** vec, i.e. a library
/// that connected cleanly and showed nothing, with no error to diagnose. (The pre-port
/// TypeScript threw on this shape — `as SubAlbum[]` then `.map` — so silently swallowing it
/// was a regression against the fix range even though it matched no earlier behaviour.)
///
/// Anything else — a string, a number, a bool where a list belongs — is an empty vec,
/// matching the `?? []` habit the TypeScript leaned on everywhere.
pub fn vec_skipping_bad<T: DeserializeOwned>(v: &Value) -> Vec<T> {
    match v {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            let mut skipped = 0usize;
            for item in items {
                match serde_json::from_value(item.clone()) {
                    Ok(parsed) => out.push(parsed),
                    Err(_) => skipped += 1,
                }
            }
            if skipped > 0 {
                warn_skipped::<T>(skipped, items.len());
            }
            out
        }
        // A single-element list collapsed into the object itself — see the docs above.
        Value::Object(_) => match serde_json::from_value(v.clone()) {
            Ok(parsed) => vec![parsed],
            Err(_) => {
                warn_skipped::<T>(1, 1);
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

/// Say out loud that a page came back short, and why.
///
/// Dropping a record is the right call — one bad record must not take down the page — but
/// doing it *silently* turns a diagnosable failure into "the library is short and nothing
/// explains it". This is the minimum that makes it diagnosable: which type, how many, out
/// of how many. A fuller diagnostic channel out of [`crate::parse`] (a count returned to
/// the caller, surfaced in the UI) is deliberately deferred; it is a design question, not a
/// line of logging.
///
/// `eprintln!` rather than `tracing` on purpose: nothing in this workspace depends on
/// `tracing`, and [`crate::client`]'s sibling crate `eko-core` already reports engine
/// errors this way. Adding a logging framework for one line would be the larger change.
fn warn_skipped<T>(skipped: usize, total: usize) {
    let full = std::any::type_name::<T>();
    let short = full.rsplit("::").next().unwrap_or(full);
    eprintln!(
        "eko-net: dropped {skipped} of {total} unreadable `{short}` record(s) from a \
         response payload; the page is short by that many rows. The usual cause is a \
         record with no usable `id`."
    );
}

/// `Option<Vec<T>>` field that skips bad elements; `null` stays `None`.
///
/// Used for the `Vec` fields nested inside object payloads (`starred2.song`,
/// `artistInfo2.similarArtist`), which reach `serde` through a whole-object
/// `from_value` and so cannot go through [`crate::parse`]'s collection reader.
pub fn opt_vec_skipping_bad<'de, D, T>(d: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        other => Ok(Some(vec_skipping_bad(&other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn u32_accepts_numbers_and_numeric_strings() {
        assert_eq!(u32_from_value(&json!(1998)), Some(1998));
        assert_eq!(u32_from_value(&json!("1998")), Some(1998));
        assert_eq!(u32_from_value(&json!(" 1998 ")), Some(1998));
        assert_eq!(u32_from_value(&json!(251.5)), Some(251), "floats truncate");
        assert_eq!(u32_from_value(&json!("251.5")), Some(251));
    }

    #[test]
    fn nonsensical_u32_values_are_absent_not_fatal() {
        for v in [
            json!(-1),
            json!(-0.5),
            json!("unknown"),
            json!(""),
            json!(null),
            json!(true),
            json!({}),
            json!([]),
            json!(5_000_000_000u64),
        ] {
            assert_eq!(u32_from_value(&v), None, "{v} should read as absent");
        }
    }

    #[test]
    fn f64_accepts_numbers_and_numeric_strings() {
        assert_eq!(f64_from_value(&json!(-7.2)), Some(-7.2));
        assert_eq!(f64_from_value(&json!("-7.2")), Some(-7.2));
        assert_eq!(f64_from_value(&json!(0)), Some(0.0));
        assert_eq!(f64_from_value(&json!("nope")), None);
        assert_eq!(f64_from_value(&json!(null)), None);
    }
}
