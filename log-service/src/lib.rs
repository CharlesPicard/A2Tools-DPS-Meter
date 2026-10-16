//! The log service: turn an uploaded Evidence Slice into the fight it records.
//!
//! One route, `POST /derive`, reachable only through a service binding from the
//! a2tools.app Worker (this Worker has no public route; see wrangler.toml). The
//! site handles accounts, storage and access; this does the one thing the site
//! cannot: run the meter's own parser, compiled to wasm32, over the slice, so a
//! log shows what the packets say rather than what a client claimed.
//!
//! Stateless and deterministic: the same slice and the same build give the
//! same record, byte for byte. That is what lets a log be re-derived later, by
//! anyone with the slice and the published source.

use a2tools_dps_meter_lib::capture::evidence_slice;
use a2tools_dps_meter_lib::rederive::{derive_decoded, Decoded, DeriveError};
use flate2::read::GzDecoder;
use ruzstd::StreamingDecoder;
use worker::*;

/// The meter's own data tables, compiled in. Without the NPC table no target
/// counts as a boss and nothing is derived; without the DoT list ticks are
/// filed as direct hits. English, because the stored record is language
/// neutral apart from names, and the site translates by code.
const NPCS: &str = include_str!("../../src/data/i18n/npcs/en.json");
const SKILLS: &str = include_str!("../../src/data/i18n/skills/en.json");
const DOTS: &str = include_str!("../../src/data/dot_skill_ids.json");

/// Upper bounds, generous against real slices (a party boss fight compresses to
/// tens or hundreds of KB) and tight against anything built to make this Worker
/// work. The second is on the fight's packets as version 2 stores them (see
/// `read_slice`), so it takes or refuses a fight alike in every version.
const MAX_GZIPPED: usize = 4 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 24 * 1024 * 1024;

/// The slice inside an upload: zstd from meters 2.0.67 on, gzip before, told
/// apart by their magic bytes, so every stored slice still derives. Read as it
/// is unpacked, never whole: version 3 stores bundles unpacked and is larger
/// so, but its records read back as version 2's, and only those are counted.
/// A bomb stops at the limit. None for anything that is not a slice.
fn read_slice(body: &[u8]) -> Option<Decoded> {
    if body.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        evidence_slice::decode_from(StreamingDecoder::new(body).ok()?, MAX_RECORD_BYTES)
    } else {
        evidence_slice::decode_from(GzDecoder::new(body), MAX_RECORD_BYTES)
    }
}

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Post || req.path() != "/derive" {
        return Response::error("not found", 404);
    }
    // Defence in depth: the binding is the real boundary, but a Worker that
    // does expensive work on request should not trust that nobody ever adds a
    // route to it by mistake.
    let key = env.secret("LOG_SERVICE_KEY").map(|s| s.to_string()).unwrap_or_default();
    let presented = req.headers().get("x-a2-service-key")?.unwrap_or_default();
    if key.is_empty() || !constant_time_eq(key.as_bytes(), presented.as_bytes()) {
        return Response::error("forbidden", 403);
    }

    let body = req.bytes().await?;
    if body.len() > MAX_GZIPPED {
        return error("too_large", "The slice is larger than any real fight.", 413);
    }
    let Some(decoded) = read_slice(&body) else {
        return error(
            "not_a_slice",
            "Not a gzipped or zstd-compressed Evidence Slice this build reads, or larger than any real fight.",
            422,
        );
    };

    // The names the upload says it showed, base64 of a JSON array, optional:
    // the client's own leak check, run again here, where a client cannot skip it.
    let names: Vec<String> = req
        .headers()
        .get("x-a2-names")?
        .and_then(|h| base64_decode(&h))
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();

    match derive_decoded(&decoded, NPCS, SKILLS, DOTS) {
        Ok(fight) => {
            let leaked = evidence_slice::leaked_names(&decoded.0, &names);
            let mut json = serde_json::to_value(&fight)?;
            json["checks"]["leakedNames"] = serde_json::json!(leaked);
            Response::from_json(&json)
        }
        Err(DeriveError::NotASlice) => error("not_a_slice", "Not an Evidence Slice this build reads.", 422),
        Err(DeriveError::NothingDerived) => error("no_fight", "The slice contains no boss fight.", 422),
    }
}

fn error(code: &str, message: &str, status: u16) -> Result<Response> {
    Ok(Response::from_json(&serde_json::json!({ "error": code, "message": message }))?
        .with_status(status))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Standard base64, padded or not. Small enough not to need a crate.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in text.trim().bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}
