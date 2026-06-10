use crate::{
    analysis,
    beautify::beautify,
    chunks,
    config::Config,
    fetcher::Fetcher,
    ingest::{AssetKind, IngestionRequest},
    optimizer,
    preanalysis,
    sourcemap,
    storage::{AssetRecord, RelationshipRecord, Storage, content_hash},
};
use anyhow::Result;
use std::sync::Arc;
use url::Url;

#[derive(Clone)]
pub struct Processor {
    config: Config,
    storage: Storage,
    fetcher: Fetcher,
}

impl Processor {
    pub async fn new(config: Config) -> Result<Self> {
        let storage = Storage::new(config.project_root()).await?;
        let fetcher = Fetcher::new(
            config.rate_per_second,
            config.rate_per_minute,
            config.fetch_concurrency,
        )?;
        Ok(Self {
            config,
            storage,
            fetcher,
        })
    }

    pub async fn process_ingestion(&self, payload: IngestionRequest) -> Result<()> {
        if payload.response.body.len() > self.config.max_body_bytes {
            anyhow::bail!("response body exceeds max size");
        }
        if !self.in_scope(&payload.request.url) {
            return Ok(());
        }
        let content_type = header_value(&payload.response.headers, "content-type");
        let kind = AssetKind::from_headers_and_body(
            content_type,
            &payload.request.url,
            &payload.response.body,
        );
        if !kind.is_processable() {
            return Ok(());
        }
        self.process_asset(
            payload.request.url,
            payload.response.body,
            kind,
            payload.request.headers,
            None,
            None,
        )
        .await
    }

    async fn process_asset(
        &self,
        url: String,
        content: String,
        kind: AssetKind,
        headers: std::collections::BTreeMap<String, String>,
        parent_url: Option<String>,
        discovered_by: Option<String>,
    ) -> Result<()> {
        // ── 1. Save original ──────────────────────────────────────────────────
        let original_path = self.storage.save_original(&url, kind, &content).await?;

        // ── 2. Beautify ───────────────────────────────────────────────────────
        let beautified_path = if matches!(kind, AssetKind::Html | AssetKind::JavaScript) {
            Some(
                self.storage
                    .save_beautified(&url, kind, &beautify(kind, &content))
                    .await?,
            )
        } else {
            None
        };

        // ── 3. Pre-analysis (fast, synchronous, no I/O itself) ────────────────
        let preanalysis_path = if matches!(kind, AssetKind::JavaScript) {
            let pa = preanalysis::preanalyze(&url, &content);

            if let Some(ref fw) = pa.framework {
                tracing::debug!(url=%url, framework=?fw, "framework detected");
            }
            if let Some(ref bundler) = pa.bundler {
                tracing::debug!(url=%url, bundler=?bundler, "bundler detected");
            }

            match serde_json::to_string_pretty(&pa) {
                Ok(json) => {
                    match self.storage.save_preanalysis_json(&url, &json).await {
                        Ok(p) => {
                            if let Err(e) = self.storage.append_preanalysis(&pa).await {
                                tracing::debug!(%e, "failed to append preanalysis record");
                            }
                            Some(p.display().to_string())
                        }
                        Err(e) => {
                            tracing::debug!(%e, "failed to save preanalysis json");
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(%e, "failed to serialize preanalysis");
                    None
                }
            }
        } else {
            None
        };

        // ── 4. Optimize ───────────────────────────────────────────────────────
        let optimized_path = if matches!(kind, AssetKind::JavaScript) {
            let result = optimizer::optimize(&content);

            if !result.obfuscation_hints.is_empty() {
                tracing::debug!(
                    url=%url,
                    hints=%result.obfuscation_hints.len(),
                    "obfuscation hints found"
                );
            }

            match self.storage.save_optimized(&url, &result.content).await {
                Ok(p) => Some(p.display().to_string()),
                Err(e) => {
                    tracing::debug!(%e, "failed to save optimized js");
                    None
                }
            }
        } else {
            None
        };

        // ── 5. Append asset record ────────────────────────────────────────────
        self.storage
            .append_asset(&AssetRecord {
                url: url.clone(),
                kind,
                original_path: original_path.display().to_string(),
                beautified_path: beautified_path.as_ref().map(|p| p.display().to_string()),
                sha256: content_hash(&content),
                parent_url: parent_url.clone(),
                discovered_by,
                headers: headers.clone(),
                optimized_path,
                preanalysis_path,
            })
            .await?;

        // ── 6. Relationship ───────────────────────────────────────────────────
        if let Some(parent_url) = parent_url {
            self.storage
                .append_relationship(&RelationshipRecord {
                    parent_url,
                    child_url: url.clone(),
                    relationship: "loads".to_string(),
                })
                .await?;
        }

        // ── 7. Static analysis ────────────────────────────────────────────────
        if matches!(kind, AssetKind::JavaScript) {
            self.analyze(&url, &content).await?;
        }

        // ── 8. Source maps (with VLQ reports) ────────────────────────────────
        if matches!(kind, AssetKind::Html | AssetKind::JavaScript) {
            self.process_sourcemaps(&url, &content, &headers).await?;
        }

        // ── 9. Chunk discovery ────────────────────────────────────────────────
        if matches!(kind, AssetKind::Html | AssetKind::JavaScript) {
            self.process_chunks(&url, &content, &headers).await?;
        }

        Ok(())
    }

    async fn analyze(&self, url: &str, content: &str) -> Result<()> {
        let findings = analysis::analyze(url, content);
        let json = serde_json::to_string_pretty(&findings)?;
        self.storage.save_analysis(url, &json).await?;
        for finding in findings {
            self.storage.append_finding(&finding).await?;
        }
        Ok(())
    }

    async fn process_sourcemaps(
        &self,
        asset_url: &str,
        content: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        for candidate in sourcemap::detect(asset_url, content, headers)? {
            let map_content = match candidate.content {
                Some(content) => content,
                None => match self.fetcher.get(&candidate.url, headers).await {
                    Ok(Some(content)) => content,
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::debug!(
                            %error,
                            sourcemap_url = %candidate.url,
                            "failed to fetch source map"
                        );
                        continue;
                    }
                },
            };

            if let Err(e) = self.storage.save_raw_sourcemap(&candidate.url, &map_content).await {
                tracing::debug!(%e, "failed to save raw sourcemap");
                continue;
            }

            let host = Url::parse(asset_url)
                .ok()
                .and_then(|url| url.host_str().map(ToString::to_string))
                .unwrap_or_else(|| "unknown".to_string());

            // Fast path: recover sourcesContent (no VLQ required)
            match sourcemap::reverse(&map_content) {
                Ok(sources) => {
                    for source in sources {
                        if let Err(e) = self
                            .storage
                            .save_reversed_source(&host, &source.name, &source.content)
                            .await
                        {
                            tracing::debug!(%e, source=%source.name, "failed to save reversed source");
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(%e, "failed to reverse sourcemap");
                }
            }

            // Full VLQ decode + per-file + combined reports (best-effort)
            match sourcemap::decode(&map_content) {
                Ok(decoded) => {
                    let flat = sourcemap::build_flat_mappings(&decoded);

                    let map_slug = Url::parse(&candidate.url)
                        .ok()
                        .and_then(|u| {
                            u.path_segments()
                                .and_then(|mut s| s.next_back().map(String::from))
                        })
                        .unwrap_or_else(|| "unknown.map".to_string());

                    for (source_name, report) in
                        sourcemap::format_per_file_reports(&decoded, &flat)
                    {
                        if let Err(e) = self
                            .storage
                            .save_sourcemap_per_file_report(
                                &host,
                                &map_slug,
                                &source_name,
                                &report,
                            )
                            .await
                        {
                            tracing::debug!(%e, "failed to save per-file sourcemap report");
                        }
                    }

                    let combined = sourcemap::format_combined_report(&decoded, &flat);
                    if let Err(e) = self
                        .storage
                        .save_sourcemap_combined_report(&candidate.url, &combined)
                        .await
                    {
                        tracing::debug!(%e, "failed to save combined sourcemap report");
                    }

                    tracing::debug!(
                        map_url=%candidate.url,
                        sources=%decoded.sources.len(),
                        mappings=%flat.len(),
                        "VLQ sourcemap decoded"
                    );
                }
                Err(e) => {
                    tracing::debug!(%e, map_url=%candidate.url, "VLQ decode failed, skipping reports");
                }
            }
        }
        Ok(())
    }

    async fn process_chunks(
        &self,
        asset_url: &str,
        content: &str,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        let chunk_urls = chunks::discover(asset_url, content)?;
        let processor = Arc::new(self.clone());
        for chunk_url in chunk_urls {
            let processor = Arc::clone(&processor);
            let headers = headers.clone();
            let parent = asset_url.to_string();
            match processor.fetcher.get(&chunk_url, &headers).await {
                Ok(Some(content)) => {
                    Box::pin(processor.process_asset(
                        chunk_url,
                        content,
                        AssetKind::JavaScript,
                        headers,
                        Some(parent),
                        Some("chunk-discovery".to_string()),
                    ))
                    .await?;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(%error, %chunk_url, "failed to fetch discovered chunk");
                }
            }
        }
        Ok(())
    }

    fn in_scope(&self, url: &str) -> bool {
        self.config.scope_patterns.is_empty()
            || self
                .config
                .scope_patterns
                .iter()
                .any(|pattern| wildcard_match(pattern, url))
    }
}

fn header_value<'a>(
    headers: &'a std::collections::BTreeMap<String, String>,
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return value.contains(pattern);
    }
    let mut remaining = value;
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if let Some(index) = remaining.find(part) {
            remaining = &remaining[index + part.len()..];
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{HttpRequestInfo, HttpResponseInfo};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn processes_javascript_ingestion() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config {
            output_root: Some(temp.path().to_path_buf()),
            project: "case".to_string(),
            rate_per_second: 0,
            ..Config::default()
        };
        let processor = Processor::new(config).await.unwrap();
        let payload = IngestionRequest {
            request: HttpRequestInfo {
                method: "GET".to_string(),
                url: "https://example.com/static/app.js".to_string(),
                headers: BTreeMap::new(),
            },
            response: HttpResponseInfo {
                status: 200,
                headers: BTreeMap::from([(
                    "Content-Type".to_string(),
                    "application/javascript".to_string(),
                )]),
                body: "fetch('/api'); localStorage.setItem('x','y')".to_string(),
            },
        };

        processor.process_ingestion(payload).await.unwrap();

        assert!(
            temp.path()
                .join("case/original/example.com/static/app.js")
                .exists()
        );
        assert!(
            temp.path()
                .join("case/beautified/example.com/static/app.js")
                .exists()
        );
        assert!(
            temp.path()
                .join("case/optimized/example.com/static/app.js")
                .exists()
        );
        assert!(temp
            .path()
            .join("case/preanalysis/example.com/static")
            .exists());
        assert!(
            temp.path()
                .join("case/analysis/example.com/static/app.analysis.json")
                .exists()
        );
        assert!(temp.path().join("case/metadata/findings.jsonl").exists());
        assert!(temp.path().join("case/metadata/preanalysis.jsonl").exists());
    }
}
