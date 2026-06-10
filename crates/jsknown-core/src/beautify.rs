use crate::ingest::AssetKind;

pub fn beautify(kind: AssetKind, content: &str) -> String {
    match kind {
        AssetKind::Html => beautify_html(content),
        AssetKind::JavaScript => beautify_javascript(content),
        _ => content.to_string(),
    }
}

// ── HTML beautifier ───────────────────────────────────────────────────────────

fn beautify_html(content: &str) -> String {
    let mut out = String::new();
    let mut indent = 0usize;
    let mut i = 0usize;
    let bytes = content.as_bytes();

    while i < content.len() {
        if bytes[i] != b'<' {
            let next = content[i..]
                .find('<')
                .map(|offset| i + offset)
                .unwrap_or(content.len());
            emit_text(&mut out, indent, &content[i..next]);
            i = next;
            continue;
        }

        if content[i..].starts_with("<!--") {
            let end = content[i..]
                .find("-->")
                .map(|offset| i + offset + 3)
                .unwrap_or(content.len());
            emit_line(&mut out, indent, content[i..end].trim());
            i = end;
            continue;
        }

        let Some(tag_end) = find_tag_end(content, i) else {
            emit_text(&mut out, indent, &content[i..]);
            break;
        };

        let tag = content[i..=tag_end].trim();
        let tag_name = html_tag_name(tag);
        let is_closing = tag.starts_with("</");
        let is_void = tag.ends_with("/>") || tag_name.as_deref().is_some_and(is_void_html_tag);

        if is_closing {
            indent = indent.saturating_sub(1);
        }

        emit_line(&mut out, indent, tag);
        i = tag_end + 1;

        if matches!(tag_name.as_deref(), Some("script" | "style")) && !is_closing {
            let close = format!("</{}>", tag_name.as_deref().unwrap());
            let lower_tail = content[i..].to_ascii_lowercase();
            if let Some(close_offset) = lower_tail.find(&close) {
                let raw_body = &content[i..i + close_offset];
                let body = if tag_name.as_deref() == Some("script") {
                    beautify_javascript(raw_body)
                } else {
                    beautify_css_like(raw_body)
                };
                for line in body.lines().filter(|line| !line.trim().is_empty()) {
                    emit_line(&mut out, indent + 1, line.trim_end());
                }
                emit_line(
                    &mut out,
                    indent,
                    &content[i + close_offset..i + close_offset + close.len()],
                );
                i += close_offset + close.len();
                continue;
            }
        }

        if !is_closing && !is_void && !tag.starts_with("<!") {
            indent += 1;
        }
    }

    collapse_blank_lines(&out)
}

fn find_tag_end(content: &str, start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let bytes = content.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if matches!(b, b'"' | b'\'') {
            quote = Some(b);
        } else if b == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn html_tag_name(tag: &str) -> Option<String> {
    let tag = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .trim_start_matches('!')
        .trim();
    let name: String = tag
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

fn is_void_html_tag(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn emit_text(out: &mut String, indent: usize, text: &str) {
    for line in text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        emit_line(out, indent, line.trim());
    }
}

fn emit_line(out: &mut String, indent: usize, line: &str) {
    if line.trim().is_empty() {
        return;
    }
    out.push_str(&"  ".repeat(indent));
    out.push_str(line.trim());
    out.push('\n');
}

fn collapse_blank_lines(content: &str) -> String {
    let mut out = String::new();
    let mut previous_blank = false;
    for line in content.lines() {
        let blank = line.trim().is_empty();
        if blank && previous_blank {
            continue;
        }
        previous_blank = blank;
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn beautify_css_like(content: &str) -> String {
    let mut out = String::new();
    let mut indent = 0usize;
    for ch in content.chars() {
        match ch {
            '{' => {
                out.push_str(" {\n");
                indent += 1;
                out.push_str(&"  ".repeat(indent));
            }
            '}' => {
                indent = indent.saturating_sub(1);
                trim_trailing_space(&mut out);
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
                out.push('}');
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
            }
            ';' => {
                out.push(';');
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
            }
            _ => out.push(ch),
        }
    }
    out
}

// ── JavaScript token-aware beautifier ────────────────────────────────────────

/// Minimal token types for the JS beautifier.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// A string literal including both quote characters and all internal escapes (verbatim).
    Str(String),
    /// A template literal including backticks and all `${...}` expressions (verbatim).
    Template(String),
    /// A regex literal including slashes and flags (verbatim). Only emitted when can_regex is true.
    Regex(String),
    /// A line comment `// ...` (verbatim, no trailing newline).
    LineComment(String),
    /// A block comment `/* ... */` (verbatim).
    BlockComment(String),
    /// Any run of identifier/keyword/number characters.
    Word(String),
    /// `{` `[` `(` `)` `]` `}` `;` `,` — single meaningful punctuation characters.
    Punct(char),
    /// Multi-character operators: `=>`, `===`, `!==`, `>=`, `<=`, `**`, `&&`, `||`, `??`, `++`,
    /// `--`, `+=`, `-=`, `*=`, `/=`, `%=`, `**=`, `&&=`, `||=`, `??=`, `...`.
    Op(String),
}

/// Keywords after which a `/` starts a regex rather than a division operator.
const REGEX_STARTERS: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "void",
    "delete",
    "throw",
    "new",
    "in",
    "of",
    "case",
    "yield",
    "await",
];

fn tokenize(content: &str) -> Vec<Tok> {
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut tokens: Vec<Tok> = Vec::new();
    // After seeing one of these tokens, '/' starts a regex instead of division.
    let mut can_regex = true;

    macro_rules! peek {
        ($offset:expr) => {
            chars.get(i + $offset).copied()
        };
    }

    while i < len {
        let ch = chars[i];

        // ── skip plain whitespace (regenerated by printer) ────────────────────
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // ── line comment ──────────────────────────────────────────────────────
        if ch == '/' && peek!(1) == Some('/') {
            let start = i;
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            tokens.push(Tok::LineComment(chars[start..i].iter().collect()));
            can_regex = false;
            continue;
        }

        // ── block comment ─────────────────────────────────────────────────────
        if ch == '/' && peek!(1) == Some('*') {
            let start = i;
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2; // consume */
            tokens.push(Tok::BlockComment(chars[start..i].iter().collect()));
            // block comments don't change regex context
            continue;
        }

        // ── regex literal ─────────────────────────────────────────────────────
        if ch == '/' && can_regex {
            let start = i;
            i += 1;
            let mut in_char_class = false;
            while i < len {
                let c = chars[i];
                if c == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if c == '[' {
                    in_char_class = true;
                } else if c == ']' {
                    in_char_class = false;
                } else if c == '/' && !in_char_class {
                    i += 1;
                    break;
                } else if c == '\n' {
                    // unterminated regex — treat as division, backtrack
                    i = start + 1;
                    tokens.push(Tok::Punct('/'));
                    break;
                }
                i += 1;
            }
            // consume flags
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Tok::Regex(chars[start..i].iter().collect()));
            can_regex = false;
            continue;
        }

        // ── string literal ────────────────────────────────────────────────────
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let start = i;
            i += 1;
            while i < len {
                let c = chars[i];
                if c == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if c == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(Tok::Str(chars[start..i].iter().collect()));
            can_regex = false;
            continue;
        }

        // ── template literal (preserve verbatim including ${...} expressions) ─
        if ch == '`' {
            let start = i;
            i += 1;
            let mut depth: i32 = 0; // tracks nesting inside ${ }
            while i < len {
                let c = chars[i];
                if c == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if c == '$' && peek!(1) == Some('{') {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if c == '}' && depth > 0 {
                    depth -= 1;
                    i += 1;
                    continue;
                }
                if c == '`' && depth == 0 {
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(Tok::Template(chars[start..i].iter().collect()));
            can_regex = false;
            continue;
        }

        // ── multi-char operators ──────────────────────────────────────────────
        {
            let two: String = chars[i..len.min(i + 2)].iter().collect();
            let three: String = chars[i..len.min(i + 3)].iter().collect();

            if three == "**="
                || three == "&&="
                || three == "||="
                || three == "??="
                || three == "..."
            {
                tokens.push(Tok::Op(three));
                i += 3;
                can_regex = true;
                continue;
            }
            if matches!(
                two.as_str(),
                "=>" | "==="
                    | "!=="
                    | ">="
                    | "<="
                    | "**"
                    | "&&"
                    | "||"
                    | "??"
                    | "++"
                    | "--"
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "=="
                    | "!="
                    | "<<"
                    | ">>"
            ) {
                let is_value = matches!(two.as_str(), "++" | "--");
                tokens.push(Tok::Op(two));
                i += 2;
                can_regex = !is_value;
                continue;
            }
        }

        // ── single punctuation that matters to the printer ────────────────────
        if matches!(ch, '{' | '}' | '[' | ']' | '(' | ')' | ';' | ',') {
            tokens.push(Tok::Punct(ch));
            can_regex = matches!(ch, '{' | '[' | '(' | ',' | ';');
            i += 1;
            continue;
        }

        if matches!(
            ch,
            '=' | '+' | '-' | '*' | '/' | '%' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
        ) {
            tokens.push(Tok::Op(ch.to_string()));
            can_regex = !matches!(ch, '?' | ':');
            i += 1;
            continue;
        }

        // ── word / identifier / number ────────────────────────────────────────
        if ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '#' {
            let start = i;
            while i < len
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '$'
                    || chars[i] == '#'
                    || chars[i] == '.')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let is_keyword = REGEX_STARTERS.contains(&word.as_str())
                || matches!(
                    word.as_str(),
                    "if" | "else"
                        | "for"
                        | "while"
                        | "do"
                        | "switch"
                        | "try"
                        | "catch"
                        | "finally"
                        | "class"
                        | "function"
                        | "async"
                        | "const"
                        | "let"
                        | "var"
                        | "import"
                        | "export"
                        | "default"
                        | "extends"
                        | "static"
                );
            can_regex = REGEX_STARTERS.contains(&word.as_str())
                || matches!(word.as_str(), "else" | "do" | "finally");
            tokens.push(Tok::Word(word));
            let _ = is_keyword;
            continue;
        }

        // ── anything else (operators, colon, dot, etc.) ───────────────────────
        tokens.push(Tok::Punct(ch));
        can_regex = matches!(
            ch,
            '=' | '!'
                | '<'
                | '>'
                | '+'
                | '-'
                | '*'
                | '/'
                | '%'
                | '&'
                | '|'
                | '^'
                | '~'
                | '?'
                | ':'
                | '.'
        );
        i += 1;
    }

    tokens
}

/// Keywords that should have a space after them when followed by `(` or `{`.
const SPACE_AFTER_KW: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "try",
    "catch",
    "finally",
    "return",
    "typeof",
    "instanceof",
    "void",
    "delete",
    "throw",
    "new",
    "in",
    "of",
    "case",
    "yield",
    "await",
    "class",
    "function",
    "async",
    "const",
    "let",
    "var",
    "import",
    "export",
    "extends",
    "static",
    "default",
];

fn print_tokens(tokens: &[Tok]) -> String {
    let mut out = String::with_capacity(tokens.len() * 4);
    let mut indent = 0usize;
    let mut paren_depth = 0usize; // inside () — commas here are arg separators, not stmt separators

    fn push_newline(out: &mut String, indent: usize) {
        // Trim trailing whitespace from the current line before adding newline
        while out.ends_with(' ') {
            out.pop();
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&"  ".repeat(indent));
    }

    for (idx, tok) in tokens.iter().enumerate() {
        let next = tokens.get(idx + 1);

        match tok {
            Tok::Str(s) | Tok::Template(s) | Tok::Regex(s) => {
                out.push_str(s);
            }

            Tok::LineComment(s) => {
                out.push_str(s);
                push_newline(&mut out, indent);
            }

            Tok::BlockComment(s) => {
                out.push_str(s);
                out.push(' ');
            }

            Tok::Word(w) => {
                out.push_str(w);
                // Add space after keywords when the next non-trivial token warrants it
                if SPACE_AFTER_KW.contains(&w.as_str())
                    && !matches!(next, Some(Tok::Punct(';')) | Some(Tok::Punct(',')) | None)
                {
                    out.push(' ');
                }
            }

            Tok::Op(op) => match op.as_str() {
                "++" | "--" | "!" | "~" => out.push_str(op),
                ":" => {
                    trim_trailing_space(&mut out);
                    out.push_str(": ");
                }
                "?" => out.push_str(" ? "),
                "." => out.push('.'),
                _ => {
                    ensure_single_space(&mut out);
                    out.push_str(op);
                    out.push(' ');
                }
            },

            Tok::Punct(ch) => match ch {
                '{' | '[' => {
                    out.push(*ch);
                    indent += 1;
                    push_newline(&mut out, indent);
                }
                '(' => {
                    out.push('(');
                    paren_depth += 1;
                }
                ')' => {
                    paren_depth = paren_depth.saturating_sub(1);
                    out.push(')');
                }
                '}' | ']' => {
                    indent = indent.saturating_sub(1);
                    push_newline(&mut out, indent);
                    out.push(*ch);
                    // After `}` or `]` we usually need a newline unless followed by specific tokens
                    match next {
                        Some(Tok::Punct(';'))
                        | Some(Tok::Punct(','))
                        | Some(Tok::Punct(')'))
                        | Some(Tok::Punct(']'))
                        | Some(Tok::Punct('}'))
                        | Some(Tok::Op(_))
                        | None => {}
                        _ => {
                            push_newline(&mut out, indent);
                        }
                    }
                }
                ';' => {
                    out.push(';');
                    push_newline(&mut out, indent);
                }
                ',' => {
                    out.push(',');
                    if paren_depth == 0 {
                        push_newline(&mut out, indent);
                    } else {
                        out.push(' ');
                    }
                }
                _ => {
                    out.push(*ch);
                }
            },
        }
    }

    // Ensure file ends with a newline
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }

    out
}

fn ensure_single_space(out: &mut String) {
    if !out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty() {
        out.push(' ');
    }
}

fn trim_trailing_space(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
}

fn beautify_javascript(content: &str) -> String {
    let tokens = tokenize(content);
    print_tokens(&tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_template_literal() {
        let src = "const x=`hello ${name}`;";
        let out = beautify(AssetKind::JavaScript, src);
        assert!(out.contains("`hello ${name}`"), "got: {out}");
    }

    #[test]
    fn preserves_string_contents() {
        let src = r#"const x="hello\nworld";"#;
        let out = beautify(AssetKind::JavaScript, src);
        assert!(out.contains(r#""hello\nworld""#), "got: {out}");
    }

    #[test]
    fn indents_blocks() {
        let src = "function f(){return 1;}";
        let out = beautify(AssetKind::JavaScript, src);
        assert!(out.contains('\n'), "expected newlines in: {out}");
        assert!(out.contains("  "), "expected indentation in: {out}");
    }

    #[test]
    fn preserves_regex() {
        let src = "var re=/[a-z]+/gi;";
        let out = beautify(AssetKind::JavaScript, src);
        assert!(out.contains("/[a-z]+/gi"), "got: {out}");
    }

    #[test]
    fn spaces_common_operators() {
        let out = beautify(AssetKind::JavaScript, "const x=a?b:c;obj.y=x+1;");
        assert!(
            out.contains("const x = a ? b: c;") || out.contains("const x = a ? b : c;"),
            "got: {out}"
        );
        assert!(out.contains("x + 1"), "got: {out}");
    }

    #[test]
    fn formats_script_inside_html() {
        let out = beautify(
            AssetKind::Html,
            "<html><body><script>function f(){return 1;}</script></body></html>",
        );
        assert!(out.contains("<script>"), "got: {out}");
        assert!(out.contains("function f()"), "got: {out}");
        assert!(out.contains("  <body>"), "got: {out}");
    }
}
