use crate::ingest::AssetKind;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};
use url::Url;

#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetRecord {
    pub url: String,
    pub kind: AssetKind,
    pub original_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beautified_path: Option<String>,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_by: Option<String>,
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimized_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preanalysis_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationshipRecord {
    pub parent_url: String,
    pub child_url: String,
    pub relationship: String,
}

impl Storage {
    pub async fn new(root: PathBuf) -> Result<Self> {
        for dir in [
            "original",
            "beautified",
            "optimized",
            "preanalysis",
            "sourcemaps/raw",
            "sourcemaps/reversed",
            "sourcemaps/reports/per-file",
            "sourcemaps/reports/combined",
            "analysis",
            "metadata",
        ] {
            fs::create_dir_all(root.join(dir)).await?;
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn save_original(
        &self,
        url: &str,
        kind: AssetKind,
        content: &str,
    ) -> Result<PathBuf> {
        let path = self.url_to_path("original", url, kind)?;
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn save_beautified(
        &self,
        url: &str,
        kind: AssetKind,
        content: &str,
    ) -> Result<PathBuf> {
        let path = self.url_to_path("beautified", url, kind)?;
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn save_optimized(&self, url: &str, content: &str) -> Result<PathBuf> {
        let path = self.url_to_path("optimized", url, AssetKind::JavaScript)?;
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn save_preanalysis_json(&self, url: &str, content: &str) -> Result<PathBuf> {
        let mut path = self.url_to_path("preanalysis", url, AssetKind::JavaScript)?;
        // Replace or add .preanalysis.json extension
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "asset".to_string());
        path.set_file_name(format!("{stem}.preanalysis.json"));
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn save_raw_sourcemap(&self, url: &str, content: &str) -> Result<PathBuf> {
        let path = self.url_to_path("sourcemaps/raw", url, AssetKind::SourceMap)?;
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn save_reversed_source(
        &self,
        host: &str,
        source_name: &str,
        content: &str,
    ) -> Result<PathBuf> {
        let mut path = self
            .root
            .join("sourcemaps")
            .join("reversed")
            .join(sanitize_segment(host));
        for part in sanitize_source_path(source_name) {
            path.push(part);
        }
        write_file(&path, content).await?;
        Ok(path)
    }

    /// Saves a per-source-file VLQ mapping report under sourcemaps/reports/per-file/.
    pub async fn save_sourcemap_per_file_report(
        &self,
        host: &str,
        map_slug: &str,
        source_name: &str,
        content: &str,
    ) -> Result<PathBuf> {
        let mut path = self
            .root
            .join("sourcemaps/reports/per-file")
            .join(sanitize_segment(host))
            .join(sanitize_segment(map_slug));
        for part in sanitize_source_path(source_name) {
            path.push(part);
        }
        // Ensure it ends with .mapping.txt
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "source".to_string());
        path.set_file_name(format!("{name}.mapping.txt"));
        ensure_inside(&self.root, &path)?;
        write_file(&path, content).await?;
        Ok(path)
    }

    /// Saves the combined cross-source VLQ report under sourcemaps/reports/combined/.
    pub async fn save_sourcemap_combined_report(
        &self,
        map_url: &str,
        content: &str,
    ) -> Result<PathBuf> {
        let mut path =
            self.url_to_path("sourcemaps/reports/combined", map_url, AssetKind::SourceMap)?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "map".to_string());
        path.set_file_name(format!("{stem}.combined.txt"));
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn save_analysis(&self, url: &str, content: &str) -> Result<PathBuf> {
        let mut path = self.url_to_path("analysis", url, AssetKind::JavaScript)?;
        path.set_extension("analysis.json");
        write_file(&path, content).await?;
        Ok(path)
    }

    pub async fn append_asset(&self, record: &AssetRecord) -> Result<()> {
        self.append_jsonl("assets.jsonl", record).await
    }

    pub async fn append_relationship(&self, record: &RelationshipRecord) -> Result<()> {
        self.append_jsonl("relationships.jsonl", record).await
    }

    pub async fn append_finding<T: Serialize>(&self, finding: &T) -> Result<()> {
        self.append_jsonl("findings.jsonl", finding).await
    }

    pub async fn append_preanalysis<T: Serialize>(&self, record: &T) -> Result<()> {
        self.append_jsonl("preanalysis.jsonl", record).await
    }

    async fn append_jsonl<T: Serialize>(&self, file: &str, value: &T) -> Result<()> {
        let path = self.root.join("metadata").join(file);
        let mut handle = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        let line = serde_json::to_string(value)?;
        handle.write_all(line.as_bytes()).await?;
        handle.write_all(b"\n").await?;
        Ok(())
    }

    pub fn url_to_path(&self, subfolder: &str, raw_url: &str, kind: AssetKind) -> Result<PathBuf> {
        let parsed = Url::parse(raw_url).with_context(|| format!("invalid URL: {raw_url}"))?;
        let host = parsed.host_str().context("URL has no host")?;
        let mut path = self.root.join(subfolder).join(sanitize_segment(host));
        let mut segments: Vec<String> = parsed
            .path_segments()
            .map(|parts| {
                parts
                    .filter(|part| !part.is_empty())
                    .map(sanitize_segment)
                    .collect()
            })
            .unwrap_or_default();

        if segments.is_empty()
            || raw_url.ends_with('/')
            || matches!(kind, AssetKind::Html) && parsed.path().ends_with('/')
        {
            segments.push("(index).html".to_string());
        }

        if matches!(kind, AssetKind::Html)
            && let Some(last) = segments.last_mut()
            && !last.contains('.')
        {
            *last = format!("{last}.html");
        }

        if let Some(query) = parsed.query()
            && let Some(last) = segments.last_mut()
        {
            let suffix = short_hash(query);
            *last = format!("{last}__q_{suffix}");
        }

        for segment in segments {
            path.push(segment);
        }
        ensure_inside(&self.root, &path)?;
        Ok(path)
    }
}

pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

async fn write_file(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    fs::create_dir_all(parent).await?;
    fs::write(path, content).await?;
    Ok(())
}

pub(crate) fn sanitize_segment(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '(' | ')') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('.');
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn sanitize_source_path(input: &str) -> Vec<String> {
    input
        .replace('\\', "/")
        .split('/')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() || part == "." || part == ".." || part.ends_with(':') {
                None
            } else {
                Some(sanitize_segment(part.split('?').next().unwrap_or(part)))
            }
        })
        .collect()
}

fn ensure_inside(root: &Path, path: &Path) -> Result<()> {
    let mut depth = 0isize;
    for component in path.strip_prefix(root).unwrap_or(path).components() {
        match component {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => {}
        }
        if depth < 0 {
            bail!("path escapes project root");
        }
    }
    Ok(())
}

fn short_hash(input: &str) -> String {
    content_hash(input).chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mirrors_url_path() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::new(temp.path().to_path_buf()).await.unwrap();
        let path = storage
            .url_to_path(
                "original",
                "https://example.com/assets/app.js?v=1",
                AssetKind::JavaScript,
            )
            .unwrap();
        assert!(
            path.to_string_lossy()
                .contains("example.com/assets/app.js__q_")
        );
    }

    #[tokio::test]
    async fn creates_new_dirs() {
        let temp = tempfile::tempdir().unwrap();
        Storage::new(temp.path().to_path_buf()).await.unwrap();
        assert!(temp.path().join("optimized").exists());
        assert!(temp.path().join("preanalysis").exists());
        assert!(temp.path().join("sourcemaps/reports/per-file").exists());
        assert!(temp.path().join("sourcemaps/reports/combined").exists());
    }
}
