use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeSet;
use url::Url;

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

pub fn discover(asset_url: &str, content: &str) -> Result<Vec<String>> {
    let base = Url::parse(asset_url)?;
    let mut out = BTreeSet::new();

    for cap in JS_PATH_RE
        .captures_iter(content)
        .chain(SINGLE_JS_PATH_RE.captures_iter(content))
    {
        if let Some(url) = resolve(&base, &cap[1]) {
            out.insert(url);
        }
    }

    for cap in VITE_DEPS_RE.captures_iter(content) {
        for quoted in WEBPACK_STRING_RE.captures_iter(&cap[1]) {
            if let Some(url) = resolve(&base, &quoted[1]) {
                out.insert(url);
            }
        }
    }

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

    if content.contains("_next/static") || content.contains("__BUILD_MANIFEST") {
        for cap in WEBPACK_STRING_RE.captures_iter(content) {
            if cap[1].contains("_next/static")
                && let Some(url) = resolve(&base, &cap[1])
            {
                out.insert(url);
            }
        }
    }

    Ok(out.into_iter().collect())
}

fn resolve(base: &Url, candidate: &str) -> Option<String> {
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        return Some(candidate.to_string());
    }
    if candidate.starts_with("data:") || candidate.contains("${") || candidate.contains("undefined")
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
}
