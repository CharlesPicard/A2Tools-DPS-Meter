//! Turn an uploaded Evidence Slice into the fight it records, with the meter's
//! own parser, as the log service does (`log-service/src/lib.rs`): the site
//! shows what the packets say, never what a client claimed.

use a2tools_dps_meter_lib::capture::evidence_slice;
use a2tools_dps_meter_lib::rederive::{derive_decoded, Decoded, DeriveError, DerivedFight};
use flate2::read::GzDecoder;
use ruzstd::StreamingDecoder;

/// The meter's own data tables, compiled in. English: the stored record is
/// language neutral apart from names.
const NPCS: &str = include_str!("../../src/data/i18n/npcs/en.json");
const SKILLS: &str = include_str!("../../src/data/i18n/skills/en.json");
const DOTS: &str = include_str!("../../src/data/dot_skill_ids.json");

/// As the log service takes them: generous against real slices, tight against
/// anything built to make the server work.
pub const MAX_COMPRESSED: usize = 16 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 24 * 1024 * 1024;

/// zstd from meters 2.0.67 on, gzip before, told apart by their magic bytes.
fn read_slice(body: &[u8]) -> Option<Decoded> {
    if body.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        evidence_slice::decode_from(StreamingDecoder::new(body).ok()?, MAX_RECORD_BYTES)
    } else {
        evidence_slice::decode_from(GzDecoder::new(body), MAX_RECORD_BYTES)
    }
}

#[derive(Debug)]
pub enum Failure {
    NotASlice,
    NoFight,
}

impl Failure {
    pub fn message(&self) -> &'static str {
        match self {
            Failure::NotASlice => "Not a gzipped or zstd-compressed Evidence Slice this server reads, or larger than any real fight.",
            Failure::NoFight => "The recording contains no boss fight.",
        }
    }
}

/// The fight, and how many of `names` appear in plain text in the slice.
pub fn derive(compressed: &[u8], names: &[String]) -> Result<(DerivedFight, usize), Failure> {
    let decoded = read_slice(compressed).ok_or(Failure::NotASlice)?;
    let fight = derive_decoded(&decoded, NPCS, SKILLS, DOTS).map_err(|e| match e {
        DeriveError::NothingDerived => Failure::NoFight,
        DeriveError::NotASlice => Failure::NotASlice,
    })?;
    let leaked = evidence_slice::leaked_names(&decoded.0, names);
    Ok((fight, leaked))
}

pub fn parser_version() -> String {
    a2tools_dps_meter_lib::rederive::parser_version()
}

/// The English NPC table, for boss names the site shows (timers).
pub fn npc_names() -> std::collections::HashMap<i32, String> {
    let parsed: serde_json::Value = serde_json::from_str(NPCS).unwrap_or_default();
    let mut out = std::collections::HashMap::new();
    if let Some(map) = parsed.as_object() {
        for (code, v) in map {
            let name = v.as_str().map(str::to_string).or_else(|| v.get("name").and_then(|n| n.as_str()).map(str::to_string));
            if let (Ok(code), Some(name)) = (code.parse::<i32>(), name) {
                out.insert(code, name);
            }
        }
    }
    out
}
