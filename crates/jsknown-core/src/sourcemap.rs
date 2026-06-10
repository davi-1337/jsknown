use anyhow::{Context, Result};
use base64::Engine;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

// Handles //# sourceMappingURL=, //@ sourceMappingURL= (legacy), and /* sourceMappingURL= */
static SOURCE_MAPPING_COMMENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?://[#@]|/\*)\s*sourceMappingURL=([^\s*]+)"#).unwrap());

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SourceMapCandidate {
    pub url: String,
    pub content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReversedSource {
    pub name: String,
    pub content: String,
}

/// A fully-decoded source map including parsed VLQ mappings.
#[derive(Debug, Clone, Serialize)]
pub struct SourceMapDecoded {
    pub version: u32,
    pub sources: Vec<String>,
    pub sources_content: Vec<Option<String>>,
    pub names: Vec<String>,
    /// mappings[generated_line] = list of segments on that line
    pub mappings: Vec<Vec<DecodedSegment>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecodedSegment {
    pub generated_col: u32,
    pub source_idx: Option<u32>,
    pub original_line: Option<u32>,
    pub original_col: Option<u32>,
    pub names_idx: Option<u32>,
}

/// A flat, fully-resolved mapping entry ready for reporting.
#[derive(Debug, Clone, Serialize)]
pub struct MappingEntry {
    pub generated_line: u32,
    pub generated_col: u32,
    pub source_file: String,
    pub original_line: u32,
    pub original_col: u32,
    pub name: Option<String>,
}

// ── Raw deserialization types ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawSourceMap {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default, rename = "sourcesContent")]
    sources_content: Vec<Option<String>>,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    mappings: String,
}

// ── VLQ decoder ──────────────────────────────────────────────────────────────

fn base64_digit(ch: char) -> Option<u32> {
    match ch {
        'A'..='Z' => Some(ch as u32 - b'A' as u32),
        'a'..='z' => Some(ch as u32 - b'a' as u32 + 26),
        '0'..='9' => Some(ch as u32 - b'0' as u32 + 52),
        '+' => Some(62),
        '/' => Some(63),
        _ => None,
    }
}

/// Decodes one VLQ integer from the front of `chars`, advancing the iterator.
///
/// VLQ encoding: each character encodes 5 bits of data plus a continuation bit (bit 5).
/// The lowest bit of the final decoded value is the sign bit (1 = negative).
fn decode_one_vlq(chars: &mut impl Iterator<Item = char>) -> Option<i32> {
    let mut accum: u32 = 0;
    let mut shift: u32 = 0;
    loop {
        let digit = base64_digit(chars.next()?)?;
        let cont = (digit & 0b10_0000) != 0;
        accum |= (digit & 0b01_1111) << shift;
        shift += 5;
        if !cont || shift > 30 {
            break;
        }
    }
    let negative = (accum & 1) == 1;
    let magnitude = (accum >> 1) as i32;
    Some(if negative { -magnitude } else { magnitude })
}

/// Decodes all VLQ integers from a single comma-free segment string.
fn decode_segment_fields(seg: &str) -> Vec<i32> {
    let mut chars = seg.chars();
    let mut out = Vec::with_capacity(5);
    while let Some(val) = decode_one_vlq(&mut chars) {
        out.push(val);
    }
    out
}

// ── Mapping parser ────────────────────────────────────────────────────────────

#[derive(Default)]
struct MappingState {
    source_idx: i32,
    original_line: i32,
    original_col: i32,
    names_idx: i32,
}

fn parse_mappings(raw: &str) -> Vec<Vec<DecodedSegment>> {
    let mut lines: Vec<Vec<DecodedSegment>> = Vec::new();
    let mut state = MappingState::default();

    for line_str in raw.split(';') {
        let mut generated_col: i32 = 0;
        let mut segments = Vec::new();

        for seg_str in line_str.split(',') {
            if seg_str.is_empty() {
                continue;
            }
            let fields = decode_segment_fields(seg_str);
            if fields.is_empty() {
                continue;
            }

            generated_col += fields[0];
            let gc = generated_col.max(0) as u32;

            if fields.len() >= 4 {
                state.source_idx += fields[1];
                state.original_line += fields[2];
                state.original_col += fields[3];

                let ni = if fields.len() >= 5 {
                    state.names_idx += fields[4];
                    Some(state.names_idx.max(0) as u32)
                } else {
                    None
                };

                segments.push(DecodedSegment {
                    generated_col: gc,
                    source_idx: Some(state.source_idx.max(0) as u32),
                    original_line: Some(state.original_line.max(0) as u32),
                    original_col: Some(state.original_col.max(0) as u32),
                    names_idx: ni,
                });
            } else {
                segments.push(DecodedSegment {
                    generated_col: gc,
                    source_idx: None,
                    original_line: None,
                    original_col: None,
                    names_idx: None,
                });
            }
        }

        lines.push(segments);
    }

    lines
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Detects source map candidates for an asset: comment URLs, HTTP headers, and sibling .map URL.
/// Spec-compliant: when multiple `sourceMappingURL` comments exist, only the LAST one is used.
pub fn detect(
    asset_url: &str,
    content: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<SourceMapCandidate>> {
    let base = Url::parse(asset_url)?;
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    // Collect all comment matches and take only the last (per spec)
    let comment_matches: Vec<_> = SOURCE_MAPPING_COMMENT_RE.captures_iter(content).collect();
    if let Some(cap) = comment_matches.last() {
        add_candidate(&base, &cap[1], &mut candidates, &mut seen)?;
    }

    for (key, value) in headers {
        if key.eq_ignore_ascii_case("sourcemap") || key.eq_ignore_ascii_case("x-sourcemap") {
            add_candidate(&base, value, &mut candidates, &mut seen)?;
        }
    }

    let sibling = format!("{asset_url}.map");
    if seen.insert(sibling.clone()) {
        candidates.push(SourceMapCandidate {
            url: sibling,
            content: None,
        });
    }

    Ok(candidates)
}

/// Extracts original source files from `sourcesContent` without VLQ decoding.
/// Fast path — works even when `mappings` is absent or malformed.
pub fn reverse(content: &str) -> Result<Vec<ReversedSource>> {
    let raw: RawSourceMap = serde_json::from_str(content).context("invalid source map JSON")?;
    let mut out = Vec::new();
    for (index, source) in raw.sources.iter().enumerate() {
        if let Some(Some(source_content)) = raw.sources_content.get(index) {
            out.push(ReversedSource {
                name: source.clone(),
                content: source_content.clone(),
            });
        }
    }
    Ok(out)
}

/// Fully decodes a source map JSON including VLQ mappings.
pub fn decode(map_json: &str) -> Result<SourceMapDecoded> {
    let raw: RawSourceMap = serde_json::from_str(map_json).context("invalid source map JSON")?;

    let mappings = parse_mappings(&raw.mappings);

    Ok(SourceMapDecoded {
        version: raw.version,
        sources: raw.sources,
        sources_content: raw.sources_content,
        names: raw.names,
        mappings,
    })
}

/// Flattens decoded mappings into a sorted list of fully-resolved `MappingEntry` records.
pub fn build_flat_mappings(decoded: &SourceMapDecoded) -> Vec<MappingEntry> {
    let mut entries = Vec::new();

    for (gen_line, segments) in decoded.mappings.iter().enumerate() {
        for seg in segments {
            let (Some(src_idx), Some(orig_line), Some(orig_col)) =
                (seg.source_idx, seg.original_line, seg.original_col)
            else {
                continue;
            };
            let source_file = decoded
                .sources
                .get(src_idx as usize)
                .cloned()
                .unwrap_or_else(|| format!("<source {src_idx}>"));
            let name = seg
                .names_idx
                .and_then(|ni| decoded.names.get(ni as usize).cloned());

            entries.push(MappingEntry {
                generated_line: gen_line as u32,
                generated_col: seg.generated_col,
                source_file,
                original_line: orig_line,
                original_col: orig_col,
                name,
            });
        }
    }

    entries.sort_by_key(|e| (e.generated_line, e.generated_col));
    entries
}

/// Builds a human-readable per-source-file mapping report.
/// Returns a BTreeMap keyed by source file path.
pub fn format_per_file_reports(
    _decoded: &SourceMapDecoded,
    flat: &[MappingEntry],
) -> BTreeMap<String, String> {
    // Group entries by source file
    let mut by_file: BTreeMap<String, Vec<&MappingEntry>> = BTreeMap::new();
    for entry in flat {
        by_file
            .entry(entry.source_file.clone())
            .or_default()
            .push(entry);
    }

    let mut reports = BTreeMap::new();
    for (file, entries) in by_file {
        let mut lines = Vec::with_capacity(entries.len() + 2);
        lines.push(format!("Source: {file}"));
        lines.push(format!("Mappings: {}", entries.len()));
        lines.push(String::new());
        lines.push(format!(
            "{:<8} {:<8} {:<8} {:<8} {}",
            "GenLine", "GenCol", "OrigLine", "OrigCol", "Name"
        ));
        lines.push("-".repeat(60));
        for e in &entries {
            lines.push(format!(
                "{:<8} {:<8} {:<8} {:<8} {}",
                e.generated_line + 1,
                e.generated_col,
                e.original_line + 1,
                e.original_col,
                e.name.as_deref().unwrap_or("")
            ));
        }
        reports.insert(file, lines.join("\n"));
    }
    reports
}

/// Builds a combined human-readable mapping report covering all source files.
pub fn format_combined_report(decoded: &SourceMapDecoded, flat: &[MappingEntry]) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Source Map Report — version {}", decoded.version));
    lines.push(format!("Sources: {}", decoded.sources.len()));
    lines.push(format!("Names: {}", decoded.names.len()));
    lines.push(format!("Total mappings: {}", flat.len()));
    lines.push(String::new());

    for (i, src) in decoded.sources.iter().enumerate() {
        let has_content = decoded
            .sources_content
            .get(i)
            .and_then(|c| c.as_ref())
            .is_some();
        lines.push(format!(
            "  [{i:>3}] {src}{}",
            if has_content {
                "  (has sourcesContent)"
            } else {
                ""
            }
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "{:<8} {:<8} {:<8} {:<8} {:<40} {}",
        "GenLine", "GenCol", "OrigLine", "OrigCol", "Source", "Name"
    ));
    lines.push("-".repeat(100));

    for e in flat.iter().take(50_000) {
        let short_src = if e.source_file.len() > 38 {
            format!("…{}", &e.source_file[e.source_file.len() - 37..])
        } else {
            e.source_file.clone()
        };
        lines.push(format!(
            "{:<8} {:<8} {:<8} {:<8} {:<40} {}",
            e.generated_line + 1,
            e.generated_col,
            e.original_line + 1,
            e.original_col,
            short_src,
            e.name.as_deref().unwrap_or("")
        ));
    }

    if flat.len() > 50_000 {
        lines.push(format!("… {} more entries truncated", flat.len() - 50_000));
    }

    lines.join("\n")
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn add_candidate(
    base: &Url,
    raw: &str,
    candidates: &mut Vec<SourceMapCandidate>,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    if raw.starts_with("data:") {
        if let Some((metadata, data)) = raw.split_once(',') {
            let content = if metadata.contains(";base64") {
                String::from_utf8(base64::engine::general_purpose::STANDARD.decode(data)?)?
            } else {
                urlencoding::decode(data)?.into_owned()
            };
            let synthetic_url = format!("{}.inline.map", base);
            if seen.insert(synthetic_url.clone()) {
                candidates.push(SourceMapCandidate {
                    url: synthetic_url,
                    content: Some(content),
                });
            }
        }
        return Ok(());
    }

    let resolved = base.join(raw)?.to_string();
    if seen.insert(resolved.clone()) {
        candidates.push(SourceMapCandidate {
            url: resolved,
            content: None,
        });
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverses_sources_content() {
        let reversed =
            reverse(r#"{"version":3,"sources":["src/app.js"],"sourcesContent":["alert(1)"]}"#)
                .unwrap();
        assert_eq!(reversed[0].name, "src/app.js");
    }

    #[test]
    fn detects_hash_comment() {
        let candidates = detect(
            "https://example.com/app.js",
            "//# sourceMappingURL=app.js.map",
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(
            candidates
                .iter()
                .any(|c| c.url.contains("app.js.map") && !c.url.contains(".map.map"))
        );
    }

    #[test]
    fn detects_at_comment_legacy() {
        let candidates = detect(
            "https://example.com/app.js",
            "//@ sourceMappingURL=app.js.map",
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(candidates.iter().any(|c| c.url.contains("app.js.map")));
    }

    #[test]
    fn takes_last_sourcemapping_comment() {
        // Spec: when multiple comments exist, use the last one
        let content = "//# sourceMappingURL=first.map\n//# sourceMappingURL=last.map";
        let candidates = detect("https://example.com/app.js", content, &BTreeMap::new()).unwrap();
        let urls: Vec<_> = candidates.iter().map(|c| c.url.as_str()).collect();
        // last.map should appear, first.map should not (only last is taken from comments)
        assert!(urls.iter().any(|u| u.contains("last.map")));
        assert!(!urls.iter().any(|u| u.contains("first.map")));
    }

    #[test]
    fn vlq_decode_simple() {
        // mappings "AAAA" = [0,0,0,0] (all deltas zero)
        let decoded = decode(
            r#"{"version":3,"sources":["src/a.js"],"sourcesContent":["x"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        assert_eq!(decoded.mappings.len(), 1);
        let seg = &decoded.mappings[0][0];
        assert_eq!(seg.generated_col, 0);
        assert_eq!(seg.source_idx, Some(0));
        assert_eq!(seg.original_line, Some(0));
        assert_eq!(seg.original_col, Some(0));
    }

    #[test]
    fn build_flat_mappings_sorts() {
        let decoded = decode(
            r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA,SAAA;AAAA"}"#,
        )
        .unwrap();
        let flat = build_flat_mappings(&decoded);
        for window in flat.windows(2) {
            assert!(
                (window[0].generated_line, window[0].generated_col)
                    <= (window[1].generated_line, window[1].generated_col)
            );
        }
    }
}
