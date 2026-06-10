use anyhow::{Context, Result};
use base64::Engine;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::{collections::BTreeMap, collections::BTreeSet};
use url::Url;

static SOURCE_MAPPING_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)[/#*]\s*sourceMappingURL=([^\s*]+)"#).unwrap());

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

#[derive(Debug, Deserialize)]
struct RawSourceMap {
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default, rename = "sourcesContent")]
    sources_content: Vec<Option<String>>,
}

pub fn detect(
    asset_url: &str,
    content: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<SourceMapCandidate>> {
    let base = Url::parse(asset_url)?;
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    for cap in SOURCE_MAPPING_RE.captures_iter(content) {
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
}
