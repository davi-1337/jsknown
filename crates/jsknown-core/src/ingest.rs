use anyhow::Result;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequestInfo {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponseInfo {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub body_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionRequest {
    pub request: HttpRequestInfo,
    pub response: HttpResponseInfo,
}

impl HttpResponseInfo {
    pub fn decoded_body(&self) -> Result<String> {
        let Some(encoded) = self.body_base64.as_deref().filter(|value| !value.is_empty()) else {
            return Ok(self.body.clone());
        };
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Html,
    JavaScript,
    SourceMap,
    Other,
}

impl AssetKind {
    pub fn from_headers_and_body(content_type: Option<&str>, url: &str, body: &str) -> Self {
        let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
        let lower_url = url.to_ascii_lowercase();
        if lower_url.ends_with(".map") || content_type.contains("source-map") {
            return Self::SourceMap;
        }
        if content_type.contains("html") || looks_like_html(body) {
            return Self::Html;
        }
        if content_type.contains("javascript")
            || content_type.contains("ecmascript")
            || lower_url.ends_with(".js")
            || lower_url.ends_with(".mjs")
            || looks_like_javascript(body)
        {
            return Self::JavaScript;
        }
        Self::Other
    }

    pub fn is_processable(self) -> bool {
        matches!(self, Self::Html | Self::JavaScript | Self::SourceMap)
    }
}

fn looks_like_html(body: &str) -> bool {
    let sample = body.trim_start().to_ascii_lowercase();
    sample.starts_with("<!doctype html")
        || sample.starts_with("<html")
        || sample.contains("<script")
}

fn looks_like_javascript(body: &str) -> bool {
    let sample = body.trim_start();
    sample.starts_with("(()=>")
        || sample.starts_with("(function")
        || sample.starts_with("function ")
        || sample.starts_with("import ")
        || sample.starts_with("export ")
        || sample.contains("webpackChunk")
        || sample.contains("__webpack_require__")
}
