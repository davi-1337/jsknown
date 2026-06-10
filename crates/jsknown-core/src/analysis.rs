use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub file_path: String,
    pub analyzer_name: String,
    pub value: String,
    pub start: Position,
    pub end: Position,
    pub tags: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

struct Rule {
    analyzer: &'static str,
    tag: &'static str,
    regex: Regex,
}

static RULES: Lazy<Vec<Rule>> = Lazy::new(|| {
    vec![
        rule(
            "add-event-listener",
            "event-listener",
            r#"addEventListener\s*\("#,
        ),
        rule("postmessage", "postmessage", r#"\.postMessage\s*\("#),
        rule("onmessage", "onmessage", r#"\bonmessage\s*="#),
        rule("onhashchange", "onhashchange", r#"\bonhashchange\s*="#),
        rule("eval", "javascript-injection", r#"\beval\s*\("#),
        rule("document-domain", "document-domain", r#"document\.domain"#),
        rule("window-open", "open-redirection", r#"window\.open\s*\("#),
        rule("inner-html", "dom-xss", r#"\.innerHTML\s*="#),
        rule(
            "dangerous-html",
            "react-dangerously-set-inner-html",
            r#"dangerouslySetInnerHTML"#,
        ),
        rule("fetch", "fetch", r#"\bfetch\s*\("#),
        rule(
            "fetch-options",
            "fetch-options",
            r#"\b(headers|credentials|mode|redirect)\s*:"#,
        ),
        rule(
            "http-methods",
            "http-method",
            r#"\b(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\b"#,
        ),
        rule(
            "graphql",
            "graphql",
            r#"\b(query|mutation)\s+[A-Za-z0-9_]*\s*\{"#,
        ),
        rule(
            "url-search-params",
            "url-search-params",
            r#"URLSearchParams\s*\("#,
        ),
        rule("cookie", "cookie", r#"document\.cookie"#),
        rule("local-storage", "local-storage", r#"localStorage\."#),
        rule("session-storage", "session-storage", r#"sessionStorage\."#),
        rule("window-name", "window-name", r#"window\.name"#),
        rule("location", "location", r#"(window\.)?location\."#),
        rule("hostname", "hostname", r#"https?://[A-Za-z0-9.-]+"#),
        rule("regex-pattern", "regex", r#"/[^/\n]{3,}/[gimsuy]*"#),
        rule(
            "secrets",
            "secret",
            r#"(?i)(api[_-]?key|token|secret|password)\s*[:=]\s*['"][^'"]{6,}"#,
        ),
        rule(
            "robust-paths",
            "path",
            r#"['"](/[A-Za-z0-9._~:/?#\[\]@!$&'()*+,;=%-]{2,})['"]"#,
        ),
        rule(
            "extensions",
            "is-extension",
            r#"\.(js|json|html|png|jpg|jpeg|gif|svg|webp|ico)\b"#,
        ),
    ]
});

pub fn analyze(file_path: &str, source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for rule in RULES.iter() {
        for mat in rule.regex.find_iter(source) {
            let mut tags = BTreeMap::new();
            tags.insert(rule.tag.to_string(), true);
            findings.push(Finding {
                file_path: file_path.to_string(),
                analyzer_name: rule.analyzer.to_string(),
                value: mat.as_str().to_string(),
                start: position(source, mat.start()),
                end: position(source, mat.end()),
                tags,
            });
        }
    }
    findings
}

fn rule(analyzer: &'static str, tag: &'static str, pattern: &'static str) -> Rule {
    Rule {
        analyzer,
        tag,
        regex: Regex::new(pattern).unwrap(),
    }
}

fn position(source: &str, offset: usize) -> Position {
    let mut line = 1usize;
    let mut column = 0usize;
    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    Position { line, column }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_fetch() {
        let findings = analyze("app.js", "fetch('/api')");
        assert!(
            findings
                .iter()
                .any(|finding| finding.analyzer_name == "fetch")
        );
    }
}
