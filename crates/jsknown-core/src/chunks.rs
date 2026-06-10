use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeSet;
use url::Url;

// ── Existing patterns ─────────────────────────────────────────────────────────

static JS_PATH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#""([^"]+(?:/_next/static/chunks/|/assets/|static/js/|chunk)[^"]+\.m?js[^"]*)""#)
        .unwrap()
});
static SINGLE_JS_PATH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"'([^']+(?:/_next/static/chunks/|/assets/|static/js/|chunk)[^']+\.m?js[^']*)'"#)
        .unwrap()
});
static VITE_DEPS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"__vite__mapDeps\([^)]*\[([^\]]+)\]"#).unwrap());
static WEBPACK_STRING_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"["']([^"']*\.chunk\.js|[^"']*\.js)["']"#).unwrap());

// ── New: Parcel ───────────────────────────────────────────────────────────────

static PARCEL_REQUIRE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"parcelRequire\(["']([^"']+)["']\)"#).unwrap());
static PARCEL_EXPORT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\$parcel\$(?:require|export|interopDefault)\b"#).unwrap());
// Parcel v2 bundle URL patterns
static PARCEL_BUNDLE_URL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"["']([^"']*\.(?:js|mjs))["']"#).unwrap());

// ── New: Rollup ───────────────────────────────────────────────────────────────

static ROLLUP_CHUNK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"["']([^"']+\.[a-f0-9]{8}\.js)["']"#).unwrap());

// ── New: esbuild ──────────────────────────────────────────────────────────────

static ESBUILD_CHUNK_RE: Lazy<Regex> = Lazy::new(|| {
    // esbuild uses chunk-XXXXXXXX.js pattern
    Regex::new(r#"["'](chunk-[A-Z2-7]{8}\.js)["']"#).unwrap()
});

// ── New: Angular lazy routes ──────────────────────────────────────────────────

static ANGULAR_LAZY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"loadChildren\s*:\s*\(\s*\)\s*=>\s*import\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap()
});
static ANGULAR_LAZY_STRING_RE: Lazy<Regex> = Lazy::new(|| {
    // Older Angular: loadChildren: 'module/path#ModuleName'
    Regex::new(r##"loadChildren\s*:\s*['"]([^'"#]+)["']"##).unwrap()
});

// ── New: generic dynamic import() with string literal ─────────────────────────

static DYNAMIC_IMPORT_STR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\bimport\s*\(\s*(?:/\*[^*]*\*/\s*)?['"]([^'"]+\.m?js)['"]"#).unwrap()
});

// ── New: AMD define/require with array ────────────────────────────────────────

static AMD_DEFINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdefine\s*\(\s*\[([^\]]+)\]"#).unwrap());
static AMD_REQUIRE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\brequire\s*\(\s*\[([^\]]+)\]"#).unwrap());

// ── New: SystemJS ─────────────────────────────────────────────────────────────

static SYSTEMJS_IMPORT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"System\.import\s*\(\s*['"]([^'"]+)['"]"#).unwrap());
static SYSTEMJS_REGISTER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"System\.register\s*\(\s*\[([^\]]+)\]"#).unwrap());

// ── Shared quoted-string extractor (for AMD/SystemJS arrays) ──────────────────

static QUOTED_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"["']([^"']+)["']"#).unwrap());

fn extract_quoted_from_array(array_body: &str) -> Vec<String> {
    QUOTED_RE
        .captures_iter(array_body)
        .map(|c| c[1].to_string())
        .collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn discover(asset_url: &str, content: &str) -> Result<Vec<String>> {
    let base = Url::parse(asset_url)?;
    let mut out = BTreeSet::new();

    // ── Existing: Next.js / generic chunk paths ───────────────────────────────
    for cap in JS_PATH_RE
        .captures_iter(content)
        .chain(SINGLE_JS_PATH_RE.captures_iter(content))
    {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    // ── Existing: Vite __vite__mapDeps ───────────────────────────────────────
    for cap in VITE_DEPS_RE.captures_iter(content) {
        for quoted in WEBPACK_STRING_RE.captures_iter(&cap[1]) {
            if let Some(url) = resolve(&base, &quoted[1]) {
                out.insert(url);
            }
        }
    }

    // ── Existing: Webpack chunk loading ──────────────────────────────────────
    if content.contains("Loading chunk") || content.contains("__webpack_require__.u") {
        for cap in WEBPACK_STRING_RE.captures_iter(content) {
            let value = &cap[1];
            if value.contains(".js")
                && (value.contains("chunk") || value.contains("[id]") || value.contains("[name]"))
                && let Some(url) = resolve(&base, value)
            {
                out.insert(url);
            }
        }
    }

    // ── Existing: Next.js BUILD_MANIFEST ─────────────────────────────────────
    if content.contains("_next/static") || content.contains("__BUILD_MANIFEST") {
        for cap in WEBPACK_STRING_RE.captures_iter(content) {
            if cap[1].contains("_next/static")
                && let Some(url) = resolve(&base, &cap[1])
            {
                out.insert(url);
            }
        }
    }

    // ── New: Rollup content-hash chunks ──────────────────────────────────────
    for cap in ROLLUP_CHUNK_RE.captures_iter(content) {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    // ── New: esbuild split chunks ─────────────────────────────────────────────
    for cap in ESBUILD_CHUNK_RE.captures_iter(content) {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    // ── New: Angular lazy routes ──────────────────────────────────────────────
    for cap in ANGULAR_LAZY_RE
        .captures_iter(content)
        .chain(ANGULAR_LAZY_STRING_RE.captures_iter(content))
    {
        let candidate = cap[1].split('#').next().unwrap_or(&cap[1]);
        if let Some(url) = resolve(&base, candidate) {
            out.insert(url);
        }
    }

    // ── New: Generic dynamic import() with string literal ────────────────────
    for cap in DYNAMIC_IMPORT_STR_RE.captures_iter(content) {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    // ── New: SystemJS individual imports ────────────────────────────────────
    for cap in SYSTEMJS_IMPORT_RE.captures_iter(content) {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    // ── New: AMD define/require + SystemJS register arrays ───────────────────
    for cap in AMD_DEFINE_RE
        .captures_iter(content)
        .chain(AMD_REQUIRE_RE.captures_iter(content))
        .chain(SYSTEMJS_REGISTER_RE.captures_iter(content))
    {
        for quoted in extract_quoted_from_array(&cap[1]) {
            if (quoted.ends_with(".js") || quoted.ends_with(".mjs"))
                && let Some(url) = resolve(&base, &quoted)
            {
                out.insert(url);
            }
        }
    }

    // ── New: Parcel v1 parcelRequire() ────────────────────────────────────────
    for cap in PARCEL_REQUIRE_RE.captures_iter(content) {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    // ── New: Parcel v2 — if file uses $parcel$ exports, scan for .js strings ──
    if PARCEL_EXPORT_RE.is_match(content) {
        for cap in PARCEL_BUNDLE_URL_RE.captures_iter(content) {
            let v = &cap[1];
            if !v.contains("node_modules")
                && !v.starts_with("data:")
                && let Some(url) = resolve(&base, v)
            {
                out.insert(url);
            }
        }
    }

    Ok(out.into_iter().collect())
}

fn resolve(base: &Url, candidate: &str) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        return Some(candidate.to_string());
    }
    if candidate.starts_with("data:")
        || candidate.contains("${")
        || candidate.contains("undefined")
        || candidate.is_empty()
    {
        return None;
    }
    base.join(candidate).ok().map(|url| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_next_chunks() {
        let chunks = discover(
            "https://example.com/_next/static/app.js",
            r#"self.__BUILD_MANIFEST=["/_next/static/chunks/123.js"]"#,
        )
        .unwrap();
        assert_eq!(
            chunks,
            vec!["https://example.com/_next/static/chunks/123.js"]
        );
    }

    #[test]
    fn detects_dynamic_import() {
        let chunks = discover(
            "https://example.com/app.js",
            r#"import('./components/Lazy.js')"#,
        )
        .unwrap();
        assert!(chunks.iter().any(|c| c.contains("components/Lazy.js")));
    }

    #[test]
    fn detects_angular_lazy() {
        let chunks = discover(
            "https://example.com/main.js",
            r#"loadChildren: () => import('./admin/admin.module.js')"#,
        )
        .unwrap();
        assert!(chunks.iter().any(|c| c.contains("admin.module.js")));
    }

    #[test]
    fn detects_systemjs() {
        let chunks = discover(
            "https://example.com/app.js",
            r#"System.import('https://example.com/module.js')"#,
        )
        .unwrap();
        assert!(chunks.iter().any(|c| c.contains("module.js")));
    }

    #[test]
    fn detects_amd_define() {
        let chunks = discover(
            "https://example.com/app.js",
            r#"define(['./foo.js', './bar.js'], function(foo, bar) {})"#,
        )
        .unwrap();
        assert!(chunks.iter().any(|c| c.contains("foo.js")));
        assert!(chunks.iter().any(|c| c.contains("bar.js")));
    }
}
