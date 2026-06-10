use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FrameworkKind {
    React,
    Vue,
    Angular,
    Svelte,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BundlerKind {
    Webpack,
    Vite,
    Parcel,
    Rollup,
    Esbuild,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MinifierKind {
    Terser,
    UglifyJs,
    ClosureCompiler,
    Esbuild,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObfuscationKind {
    HexPropertyAccess,
    ArrayRotation,
    StringArrayEncoding,
    HexVariableNames,
}

#[derive(Debug, Clone, Serialize)]
pub struct StringArray {
    pub var_name: String,
    pub values: Vec<String>,
    pub has_rotation_wrapper: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObfuscationHint {
    pub kind: ObfuscationKind,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptimizationResult {
    pub content: String,
    pub applied_passes: Vec<&'static str>,
    pub framework: Option<FrameworkKind>,
    pub bundler: Option<BundlerKind>,
    pub minifier: Option<MinifierKind>,
    pub string_arrays: Vec<StringArray>,
    pub obfuscation_hints: Vec<ObfuscationHint>,
}

// ── Transformation passes ─────────────────────────────────────────────────────

static VOID_ZERO_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bvoid\s+0\b").unwrap());
// Match !0 and !1 — no lookahead/lookbehind (not supported in Rust regex).
// replace_outside_strings guards against string context; we do a char-check in the pass.
static BOOL_TRUE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"!0").unwrap());
static BOOL_FALSE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"!1").unwrap());

/// Replace `void 0` with `undefined`.
fn pass_void_zero(content: &str) -> (String, bool) {
    let result = VOID_ZERO_RE.replace_all(content, "undefined");
    let changed = result != content;
    (result.into_owned(), changed)
}

/// Replace `!0` → `true` and `!1` → `false` (outside of string literals, not inside words).
fn pass_boolean_literals(content: &str) -> (String, bool) {
    let (out1, c1) = replace_bool_outside_strings(content, "!0", "true");
    let (out2, c2) = replace_bool_outside_strings(&out1, "!1", "false");
    (out2, c1 || c2)
}

/// Specialized replacement for `!0`/`!1` that also checks char boundaries.
fn replace_bool_outside_strings(content: &str, needle: &str, replacement: &str) -> (String, bool) {
    let re = if needle == "!0" { &*BOOL_TRUE_RE } else { &*BOOL_FALSE_RE };
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut in_str_flags = vec![false; len];
    let mut i = 0;
    let mut in_string: Option<char> = None;
    let mut template_depth: i32 = 0;

    while i < len {
        let ch = chars[i];
        match in_string {
            None => {
                if matches!(ch, '"' | '\'') {
                    in_string = Some(ch);
                    in_str_flags[i] = true;
                } else if ch == '`' {
                    in_string = Some('`');
                    template_depth = 0;
                    in_str_flags[i] = true;
                }
            }
            Some(q) => {
                in_str_flags[i] = true;
                if ch == '\\' {
                    i += 1;
                    if i < len { in_str_flags[i] = true; }
                } else if q == '`' {
                    if ch == '$' && i + 1 < len && chars[i + 1] == '{' {
                        template_depth += 1;
                    } else if ch == '}' && template_depth > 0 {
                        template_depth -= 1;
                    } else if ch == '`' && template_depth == 0 {
                        in_string = None;
                    }
                } else if ch == q {
                    in_string = None;
                }
            }
        }
        i += 1;
    }

    let char_byte_offsets: Vec<usize> = content
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(content.len()))
        .collect();

    let mut result = String::with_capacity(content.len());
    let mut last_end = 0;
    let mut changed = false;

    for mat in re.find_iter(content) {
        let mat_start = mat.start();
        let mat_end = mat.end();

        let char_idx = char_byte_offsets.partition_point(|&b| b < mat_start);
        let mat_char_len = content[mat_start..mat_end].chars().count();
        let any_in_str = (char_idx..char_idx + mat_char_len)
            .any(|ci| ci < in_str_flags.len() && in_str_flags[ci]);
        if any_in_str {
            result.push_str(&content[last_end..mat_end]);
            last_end = mat_end;
            continue;
        }

        // Check char boundaries: char before '!' should not be alphanumeric/dot
        let before_ok = if mat_start == 0 {
            true
        } else {
            let prev_byte = char_byte_offsets[char_idx.saturating_sub(1)];
            let prev_char = content[prev_byte..mat_start].chars().next_back();
            !matches!(prev_char, Some(c) if c.is_alphanumeric() || c == '.' || c == '_' || c == '$')
        };

        // Char after the match (the digit 0 or 1) should not be alphanumeric
        let after_ok = {
            let after_char_idx = char_idx + mat_char_len;
            if after_char_idx >= char_byte_offsets.len() - 1 {
                true
            } else {
                let after_byte = char_byte_offsets[after_char_idx];
                let after_char = content[after_byte..].chars().next();
                !matches!(after_char, Some(c) if c.is_alphanumeric() || c == '_' || c == '$')
            }
        };

        if before_ok && after_ok {
            result.push_str(&content[last_end..mat_start]);
            result.push_str(replacement);
            last_end = mat_end;
            changed = true;
        } else {
            result.push_str(&content[last_end..mat_end]);
            last_end = mat_end;
        }
    }

    result.push_str(&content[last_end..]);
    (result, changed)
}

/// Replace `\uXXXX` and `\u{XXXX}` escape sequences inside string and template literals.
fn pass_unicode_escape(content: &str) -> (String, bool) {
    let (out, changed) = decode_string_escapes(content, EscapeKind::Unicode);
    (out, changed)
}

/// Replace `\xXX` hex escape sequences inside string and template literals.
fn pass_hex_escape(content: &str) -> (String, bool) {
    let (out, changed) = decode_string_escapes(content, EscapeKind::Hex);
    (out, changed)
}

/// Fold adjacent string concatenation: `'a' + 'b'` → `'ab'` and `"a" + "b"` → `"ab"`.
/// Implemented without backreferences since Rust's regex crate doesn't support them.
fn pass_string_concat(content: &str) -> (String, bool) {
    let mut current = content.to_string();
    let mut total_changed = false;

    loop {
        let (next, changed) = fold_one_string_concat_pass(&current);
        if !changed {
            break;
        }
        total_changed = true;
        current = next;
    }

    (current, total_changed)
}

fn fold_one_string_concat_pass(content: &str) -> (String, bool) {
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    let mut changed = false;

    while i < len {
        let ch = chars[i];

        // Try to read a quoted string starting here
        if ch == '"' || ch == '\'' {
            let quote = ch;
            // Collect first string body
            let first_start = i + 1;
            let mut j = first_start;
            while j < len {
                if chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == quote {
                    break;
                }
                j += 1;
            }
            if j >= len {
                // unterminated string — emit raw and move on
                out.push(ch);
                i += 1;
                continue;
            }
            let first_body: String = chars[first_start..j].iter().collect();
            let after_first = j + 1; // position after closing quote

            // Skip whitespace + '+'
            let mut k = after_first;
            while k < len && (chars[k] == ' ' || chars[k] == '\t' || chars[k] == '\n' || chars[k] == '\r') {
                k += 1;
            }
            if k < len && chars[k] == '+' {
                k += 1;
                while k < len && (chars[k] == ' ' || chars[k] == '\t' || chars[k] == '\n' || chars[k] == '\r') {
                    k += 1;
                }
                // Must be followed by same-quote string
                if k < len && chars[k] == quote {
                    let second_start = k + 1;
                    let mut m = second_start;
                    while m < len {
                        if chars[m] == '\\' {
                            m += 2;
                            continue;
                        }
                        if chars[m] == quote {
                            break;
                        }
                        m += 1;
                    }
                    if m < len {
                        let second_body: String = chars[second_start..m].iter().collect();
                        // Emit folded string
                        out.push(quote);
                        out.push_str(&first_body);
                        out.push_str(&second_body);
                        out.push(quote);
                        i = m + 1;
                        changed = true;
                        continue;
                    }
                }
            }
            // No concat — emit the string as-is
            out.push(quote);
            out.push_str(&first_body);
            out.push(quote);
            i = after_first;
            continue;
        }

        out.push(ch);
        i += 1;
    }

    (out, changed)
}

/// Annotate obvious ternary expressions with a comment for readability.
/// Transforms `a ? b : c` at statement level into `a ? b : c /*ternary*/`.
fn pass_ternary_annotate(content: &str) -> (String, bool) {
    static TERNARY_RE: Lazy<Regex> = Lazy::new(|| {
        // Simple single-line ternary. Rust's regex crate does not support
        // lookahead, so the "already annotated" check is done in code below.
        Regex::new(r"([^?:!\s][^?:]*)\?([^:?]+):([^;{},\n]+)").unwrap()
    });

    let mut out = String::with_capacity(content.len());
    let mut last = 0;
    let mut changed = false;

    for mat in TERNARY_RE.find_iter(content) {
        out.push_str(&content[last..mat.end()]);
        let tail = &content[mat.end()..];
        if !tail.trim_start().starts_with("/*ternary*/") {
            out.push_str(" /*ternary*/");
            changed = true;
        }
        last = mat.end();
    }
    out.push_str(&content[last..]);

    (out, changed)
}

// ── Pass runner ───────────────────────────────────────────────────────────────

const MAX_OPTIMIZE_BYTES: usize = 5_000_000; // skip heavy passes above 5 MB

/// Run all optimization passes in order and return the combined result.
pub fn optimize(content: &str) -> OptimizationResult {
    let mut current = content.to_string();
    let mut applied_passes = Vec::new();

    let run_heavy = content.len() <= MAX_OPTIMIZE_BYTES;

    let passes: &[(&'static str, fn(&str) -> (String, bool))] = &[
        ("void_zero", pass_void_zero),
        ("boolean_literals", pass_boolean_literals),
        ("unicode_escape", pass_unicode_escape),
        ("hex_escape", pass_hex_escape),
        ("string_concat", pass_string_concat),
    ];

    for (name, pass_fn) in passes {
        if !run_heavy
            && matches!(*name, "unicode_escape" | "hex_escape" | "string_concat")
        {
            continue;
        }
        let (next, changed) = pass_fn(&current);
        if changed {
            applied_passes.push(*name);
            current = next;
        }
    }

    // Ternary annotation: only on small-ish files to avoid ballooning output
    if run_heavy {
        let (next, changed) = pass_ternary_annotate(&current);
        if changed {
            applied_passes.push("ternary_annotate");
            current = next;
        }
    }

    let framework = detect_framework(content);
    let bundler = detect_bundler(content);
    let minifier = detect_minifier(content);
    let string_arrays = extract_string_arrays(content);
    let obfuscation_hints = detect_obfuscation(content);

    OptimizationResult {
        content: current,
        applied_passes,
        framework,
        bundler,
        minifier,
        string_arrays,
        obfuscation_hints,
    }
}

// ── Detection functions ───────────────────────────────────────────────────────

pub(crate) fn detect_framework(content: &str) -> Option<FrameworkKind> {
    let react_score = [
        "__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED",
        "createElement",
        "useState",
        "useEffect",
        "_jsx(",
        "jsxs(",
        "React.Component",
        "ReactDOM",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let vue_score = [
        "createApp",
        "defineComponent",
        "__VUE__",
        "__vue_component__",
        "Vue.component",
        "ref(",
        "computed(",
        "onMounted",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let angular_score = [
        "ɵɵdefineComponent",
        "ɵɵinject",
        "ViewEncapsulation",
        "NgModule",
        "ɵfac",
        "ɵprov",
        "ɵdir",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let svelte_score = [
        "SvelteComponent",
        "create_fragment",
        "detach(",
        "element(",
        "SvelteComponentDev",
        "init(",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let best = [
        (react_score, FrameworkKind::React),
        (vue_score, FrameworkKind::Vue),
        (angular_score, FrameworkKind::Angular),
        (svelte_score, FrameworkKind::Svelte),
    ]
    .into_iter()
    .max_by_key(|(score, _)| *score);

    best.and_then(|(score, kind)| if score >= 2 { Some(kind) } else { None })
}

pub(crate) fn detect_bundler(content: &str) -> Option<BundlerKind> {
    let webpack_score = [
        "__webpack_require__",
        "__webpack_modules__",
        "webpackChunk",
        "__webpack_exports__",
        "webpack/runtime",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let vite_score = [
        "import.meta.hot",
        "__vite__mapDeps",
        "/@vite/",
        "vite/preload-helper",
        "__vitePreload",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let parcel_score = [
        "parcelRequire(",
        "$parcel$require(",
        "$parcel$export(",
        "$parcel$interopDefault(",
        "parcelRequire.register(",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let rollup_score = if !content.contains("__webpack_require__")
        && !content.contains("parcelRequire")
        && !content.contains("import.meta.hot")
    {
        [
            "(function (global, factory)",
            "(function (exports)",
            "Object.defineProperty(exports,",
        ]
        .iter()
        .filter(|&&s| content.contains(s))
        .count()
    } else {
        0
    };

    let esbuild_score = [
        "__commonJS(",
        "__toESM(",
        "__toCommonJS(",
        "__esm(",
        "// node_modules/",
        "__name(",
    ]
    .iter()
    .filter(|&&s| content.contains(s))
    .count();

    let best = [
        (webpack_score, BundlerKind::Webpack),
        (vite_score, BundlerKind::Vite),
        (parcel_score, BundlerKind::Parcel),
        (rollup_score, BundlerKind::Rollup),
        (esbuild_score, BundlerKind::Esbuild),
    ]
    .into_iter()
    .max_by_key(|(score, _)| *score);

    best.and_then(|(score, kind)| if score >= 1 { Some(kind) } else { None })
}

pub(crate) fn detect_minifier(content: &str) -> Option<MinifierKind> {
    if content.contains("$jscomp")
        || content.contains("goog.module")
        || content.contains("COMPILED")
    {
        return Some(MinifierKind::ClosureCompiler);
    }

    let esbuild_score = ["__commonJS(", "__toESM(", "// node_modules/", "__name("]
        .iter()
        .filter(|&&s| content.contains(s))
        .count();
    if esbuild_score >= 2 {
        return Some(MinifierKind::Esbuild);
    }

    // Terser/UglifyJS — look for license comment preservation + tiny variable names
    // Terser uses `/*! ... */` style license comments
    if content.contains("/*!") {
        return Some(MinifierKind::Terser);
    }

    None
}

pub(crate) fn detect_obfuscation(content: &str) -> Vec<ObfuscationHint> {
    static HEX_PROP_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"\[["']\\x[0-9a-fA-F]{2}"#).unwrap());
    static HEX_VAR_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b_0x[a-fA-F0-9]{3,}\b").unwrap());
    static STR_ARRAY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"var\s+_0x[a-fA-F0-9]+\s*=\s*\[['"]"#).unwrap()
    });

    let mut hints = Vec::new();

    if HEX_PROP_RE.is_match(content) {
        hints.push(ObfuscationHint {
            kind: ObfuscationKind::HexPropertyAccess,
            description: "Property access via hex-encoded string literals detected".to_string(),
        });
    }

    if content.contains("arr.push(arr.shift())")
        || content.contains(".push(.shift())")
    {
        hints.push(ObfuscationHint {
            kind: ObfuscationKind::ArrayRotation,
            description: "Array rotation pattern (push/shift) detected — common in obfuscators"
                .to_string(),
        });
    }

    if STR_ARRAY_RE.is_match(content) {
        hints.push(ObfuscationHint {
            kind: ObfuscationKind::StringArrayEncoding,
            description: "Obfuscated string array variable detected (_0x... = ['...'])".to_string(),
        });
    }

    let hex_var_count = HEX_VAR_RE.find_iter(content).count();
    if hex_var_count >= 5 {
        hints.push(ObfuscationHint {
            kind: ObfuscationKind::HexVariableNames,
            description: format!(
                "High density of hex-named identifiers ({hex_var_count} occurrences) — likely obfuscated"
            ),
        });
    }

    hints
}

pub(crate) fn extract_string_arrays(content: &str) -> Vec<StringArray> {
    static ARRAY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?:var|let|const)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*\[((?:"[^"]*"|'[^']*')(?:\s*,\s*(?:"[^"]*"|'[^']*'))*)\]"#,
        )
        .unwrap()
    });
    static QUOTED_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"["']([^"']*)["']"#).unwrap());

    let mut arrays = Vec::new();

    for cap in ARRAY_RE.captures_iter(content) {
        let var_name = cap[1].to_string();
        let body = &cap[2];
        let values: Vec<String> = QUOTED_RE
            .captures_iter(body)
            .map(|c| c[1].to_string())
            .collect();

        if values.len() < 3 {
            continue;
        }

        // Check within ±500 chars of the array end for a push/shift rotation wrapper
        let match_end = cap.get(0).map(|m| m.end()).unwrap_or(0);
        let search_end = (match_end + 500).min(content.len());
        let nearby = &content[match_end.min(content.len())..search_end];
        let has_rotation_wrapper = nearby.contains("push") && nearby.contains("shift");

        arrays.push(StringArray {
            var_name,
            values,
            has_rotation_wrapper,
        });
    }

    arrays
}

// ── Internal utilities ────────────────────────────────────────────────────────

#[derive(Copy, Clone)]
enum EscapeKind {
    Unicode,
    Hex,
}

/// Walk the content character-by-character, tracking string/template literal context.
/// When inside a string, decode escape sequences of the given kind.
fn decode_string_escapes(content: &str, kind: EscapeKind) -> (String, bool) {
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    let mut changed = false;

    // string context: None = outside, Some(q) = inside a string with quote char q,
    // Some('`') = inside a template literal
    let mut in_string: Option<char> = None;
    let mut template_depth: i32 = 0; // tracks ${ nesting inside template literals

    while i < len {
        let ch = chars[i];

        match in_string {
            None => {
                match ch {
                    '"' | '\'' => {
                        in_string = Some(ch);
                        out.push(ch);
                        i += 1;
                    }
                    '`' => {
                        in_string = Some('`');
                        template_depth = 0;
                        out.push(ch);
                        i += 1;
                    }
                    _ => {
                        out.push(ch);
                        i += 1;
                    }
                }
            }

            Some(q) => {
                if ch == '\\' && i + 1 < len {
                    let next = chars[i + 1];
                    match kind {
                        EscapeKind::Unicode if next == 'u' => {
                            // \uXXXX or \u{XXXXX}
                            if i + 2 < len && chars[i + 2] == '{' {
                                // \u{XXXX}
                                let start = i + 3;
                                let mut end = start;
                                while end < len && chars[end] != '}' {
                                    end += 1;
                                }
                                let hex: String = chars[start..end].iter().collect();
                                if let Some(cp) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                    if cp.is_ascii_graphic() || cp == ' ' {
                                        out.push(cp);
                                        changed = true;
                                        i = end + 1; // skip closing }
                                        continue;
                                    }
                                }
                            } else if i + 5 < len {
                                // \uXXXX
                                let hex: String = chars[i + 2..i + 6].iter().collect();
                                if let Some(cp) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                    if cp.is_ascii_graphic() || cp == ' ' {
                                        out.push(cp);
                                        changed = true;
                                        i += 6;
                                        continue;
                                    }
                                }
                            }
                            out.push(ch);
                            out.push(next);
                            i += 2;
                        }

                        EscapeKind::Hex if next == 'x' && i + 3 < len => {
                            // \xXX
                            let hex: String = chars[i + 2..i + 4].iter().collect();
                            if let Some(cp) = u8::from_str_radix(&hex, 16).ok().map(|b| b as char) {
                                if cp.is_ascii_graphic() || cp == ' ' {
                                    out.push(cp);
                                    changed = true;
                                    i += 4;
                                    continue;
                                }
                            }
                            out.push(ch);
                            out.push(next);
                            i += 2;
                        }

                        _ => {
                            out.push(ch);
                            out.push(next);
                            i += 2;
                        }
                    }
                    continue;
                }

                // Template literal exit/entry tracking
                if q == '`' {
                    if ch == '$' && i + 1 < len && chars[i + 1] == '{' {
                        template_depth += 1;
                        out.push(ch);
                        out.push('{');
                        i += 2;
                        continue;
                    }
                    if ch == '}' && template_depth > 0 {
                        template_depth -= 1;
                        out.push(ch);
                        i += 1;
                        continue;
                    }
                    if ch == '`' && template_depth == 0 {
                        in_string = None;
                        out.push(ch);
                        i += 1;
                        continue;
                    }
                } else if ch == q {
                    in_string = None;
                    out.push(ch);
                    i += 1;
                    continue;
                }

                out.push(ch);
                i += 1;
            }
        }
    }

    (out, changed)
}

/// Apply a regex replacement only outside string/template literals.
/// For simple boolean literal replacements this is good enough via string-FSM check.
fn replace_outside_strings(content: &str, re: &Regex, replacement: &str) -> (String, bool) {
    // Build a bitset of "inside string" positions (approximate, no comment awareness)
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut in_str = vec![false; len];
    let mut i = 0;
    let mut in_string: Option<char> = None;
    let mut template_depth: i32 = 0;

    let char_byte_offsets: Vec<usize> = content
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(content.len()))
        .collect();

    while i < len {
        let ch = chars[i];
        match in_string {
            None => {
                if matches!(ch, '"' | '\'') {
                    in_string = Some(ch);
                    in_str[i] = true;
                } else if ch == '`' {
                    in_string = Some('`');
                    template_depth = 0;
                    in_str[i] = true;
                }
            }
            Some(q) => {
                in_str[i] = true;
                if ch == '\\' {
                    i += 1;
                    if i < len {
                        in_str[i] = true;
                    }
                } else if q == '`' {
                    if ch == '$' && i + 1 < len && chars[i + 1] == '{' {
                        template_depth += 1;
                    } else if ch == '}' && template_depth > 0 {
                        template_depth -= 1;
                    } else if ch == '`' && template_depth == 0 {
                        in_string = None;
                    }
                } else if ch == q {
                    in_string = None;
                }
            }
        }
        i += 1;
    }

    let mut result = String::with_capacity(content.len());
    let mut last_end = 0;
    let mut changed = false;

    for mat in re.find_iter(content) {
        // Check if any char in this match is inside a string
        let mat_start_byte = mat.start();
        let mat_end_byte = mat.end();

        // Find char index for mat_start_byte
        let char_idx = char_byte_offsets
            .partition_point(|&b| b < mat_start_byte);
        let mat_char_len = content[mat_start_byte..mat_end_byte].chars().count();

        let any_in_str = (char_idx..char_idx + mat_char_len)
            .any(|ci| ci < in_str.len() && in_str[ci]);

        if any_in_str {
            result.push_str(&content[last_end..mat_end_byte]);
            last_end = mat_end_byte;
            continue;
        }

        result.push_str(&content[last_end..mat_start_byte]);
        result.push_str(replacement);
        last_end = mat_end_byte;
        changed = true;
    }

    result.push_str(&content[last_end..]);
    (result, changed)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn void_zero_replaced() {
        let result = optimize("var x = void 0;");
        assert!(result.content.contains("undefined"));
        assert!(result.applied_passes.contains(&"void_zero"));
    }

    #[test]
    fn boolean_true_replaced() {
        let result = optimize("var t = !0, f = !1;");
        assert!(result.content.contains("true"));
        assert!(result.content.contains("false"));
    }

    #[test]
    fn unicode_escape_decoded() {
        let result = optimize(r#"var s = "\u0048ello";"#);
        assert!(result.content.contains("Hello") || result.applied_passes.contains(&"unicode_escape"));
    }

    #[test]
    fn hex_escape_decoded() {
        let result = optimize(r#"var s = "\x48ello";"#);
        assert!(result.content.contains("Hello") || result.applied_passes.contains(&"hex_escape"));
    }

    #[test]
    fn string_concat_folded() {
        let result = optimize(r#"var s = 'hel' + 'lo';"#);
        assert!(result.content.contains("'hello'") || result.applied_passes.contains(&"string_concat"));
    }

    #[test]
    fn detects_react() {
        let content = "useState(); useEffect(); createElement(); _jsx()";
        assert_eq!(detect_framework(content), Some(FrameworkKind::React));
    }

    #[test]
    fn detects_webpack() {
        let content = "__webpack_require__; __webpack_modules__";
        assert_eq!(detect_bundler(content), Some(BundlerKind::Webpack));
    }

    #[test]
    fn detects_hex_var_obfuscation() {
        let content = "_0xab12 _0xcd34 _0xef56 _0x1234 _0x5678";
        let hints = detect_obfuscation(content);
        assert!(hints.iter().any(|h| h.kind == ObfuscationKind::HexVariableNames));
    }
}
