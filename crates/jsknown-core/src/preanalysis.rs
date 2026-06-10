use crate::optimizer::{
    BundlerKind, FrameworkKind, MinifierKind, detect_bundler, detect_framework, detect_minifier,
};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

// ── Static regexes ─────────────────────────────────────────────────────────────

static FN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bfunction[\s*({]|=>\s*[{(]|\)\s*=>").unwrap());
static CLASS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bclass\s+[A-Z][A-Za-z0-9_$]*").unwrap());
static IMPORT_STMT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*import\s").unwrap());
static EXPORT_STMT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*export\s").unwrap());
static DYNAMIC_IMPORT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bimport\s*\(").unwrap());
static AMD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bdefine\s*\(\s*\[").unwrap());
static CJS_REQUIRE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\brequire\s*\(").unwrap());
static ESM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*(?:import|export)\s").unwrap());
static SRCMAP_HINT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?://[#@]|/\*)\s*sourceMappingURL=(\S+)").unwrap());

const LAZY_PATTERNS: &[(&str, &str)] = &[
    ("react-lazy", r"React\.lazy\s*\("),
    ("dynamic-import-string", r#"import\s*\(\s*['"][^'"]+['"]\s*\)"#),
    ("require-ensure", r"require\.ensure\s*\("),
    ("vue-async", r"defineAsyncComponent\s*\("),
    ("angular-loadchildren", r"loadChildren\s*:"),
    ("loadable", r"\bloadable\s*\("),
    ("lazy-load", r"\blazyLoad\s*\("),
];

// ── Public types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PreAnalysis {
    pub url: String,
    pub size_bytes: usize,
    pub line_count: usize,
    pub avg_line_length: f64,
    pub likely_minified: bool,
    pub bundler: Option<BundlerKind>,
    pub framework: Option<FrameworkKind>,
    pub minifier: Option<MinifierKind>,
    pub has_sourcemap_comment: bool,
    pub sourcemap_url_hint: Option<String>,
    pub function_count: usize,
    pub class_count: usize,
    pub import_count: usize,
    pub export_count: usize,
    pub has_lazy_loading: bool,
    pub lazy_patterns_found: Vec<&'static str>,
    pub has_dynamic_imports: bool,
    pub has_amd: bool,
    pub has_commonjs: bool,
    pub has_esm: bool,
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Synchronous, fast pre-analysis of a JS asset — pure regex counting, no I/O.
/// Called before beautify/optimize to produce early metadata.
pub fn preanalyze(url: &str, content: &str) -> PreAnalysis {
    let size_bytes = content.len();
    let line_count = content.lines().count().max(1);
    let avg_line_length = size_bytes as f64 / line_count as f64;
    let likely_minified =
        avg_line_length > 200.0 || (line_count <= 5 && size_bytes > 5_000);

    let function_count = FN_RE.find_iter(content).count();
    let class_count = CLASS_RE.find_iter(content).count();
    let import_count = IMPORT_STMT_RE.find_iter(content).count();
    let export_count = EXPORT_STMT_RE.find_iter(content).count();
    let has_dynamic_imports = DYNAMIC_IMPORT_RE.is_match(content);
    let has_amd = AMD_RE.is_match(content);
    let has_commonjs = CJS_REQUIRE_RE.is_match(content);
    let has_esm = ESM_RE.is_match(content);

    // Lazy loading pattern detection (compiled lazily per call — acceptable since
    // the outer function is only called once per asset)
    let mut lazy_patterns_found: Vec<&'static str> = Vec::new();
    for (name, pattern) in LAZY_PATTERNS {
        if Regex::new(pattern)
            .ok()
            .map(|re| re.is_match(content))
            .unwrap_or(false)
        {
            lazy_patterns_found.push(name);
        }
    }
    let has_lazy_loading = !lazy_patterns_found.is_empty();

    let sourcemap_url_hint = SRCMAP_HINT_RE
        .captures(content)
        .map(|c| c[1].to_string());
    let has_sourcemap_comment = sourcemap_url_hint.is_some();

    let bundler = detect_bundler(content);
    let framework = detect_framework(content);
    let minifier = detect_minifier(content);

    PreAnalysis {
        url: url.to_string(),
        size_bytes,
        line_count,
        avg_line_length,
        likely_minified,
        bundler,
        framework,
        minifier,
        has_sourcemap_comment,
        sourcemap_url_hint,
        function_count,
        class_count,
        import_count,
        export_count,
        has_lazy_loading,
        lazy_patterns_found,
        has_dynamic_imports,
        has_amd,
        has_commonjs,
        has_esm,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_minified() {
        let long_line = "a".repeat(5000);
        let pa = preanalyze("https://example.com/app.js", &long_line);
        assert!(pa.likely_minified);
    }

    #[test]
    fn counts_functions() {
        let src = "function foo() {} function bar() {} const x = () => {};";
        let pa = preanalyze("https://example.com/a.js", src);
        assert!(pa.function_count >= 2);
    }

    #[test]
    fn detects_sourcemap_comment() {
        let src = "var x = 1;\n//# sourceMappingURL=app.js.map";
        let pa = preanalyze("https://example.com/a.js", src);
        assert!(pa.has_sourcemap_comment);
        assert_eq!(pa.sourcemap_url_hint.as_deref(), Some("app.js.map"));
    }

    #[test]
    fn detects_esm() {
        let src = "import React from 'react';\nexport default App;";
        let pa = preanalyze("https://example.com/a.js", src);
        assert!(pa.has_esm);
    }
}
