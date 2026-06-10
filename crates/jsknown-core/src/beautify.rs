use crate::ingest::AssetKind;

pub fn beautify(kind: AssetKind, content: &str) -> String {
    match kind {
        AssetKind::Html => beautify_html(content),
        AssetKind::JavaScript => beautify_javascript(content),
        _ => content.to_string(),
    }
}

fn beautify_html(content: &str) -> String {
    let mut out = String::new();
    let mut indent = 0usize;
    let mut token = String::new();
    let mut in_tag = false;

    for ch in content.chars() {
        token.push(ch);
        if ch == '<' {
            in_tag = true;
        }
        if in_tag && ch == '>' {
            let trimmed = token.trim();
            if trimmed.starts_with("</") {
                indent = indent.saturating_sub(1);
            }
            if !trimmed.is_empty() {
                out.push_str(&"  ".repeat(indent));
                out.push_str(trimmed);
                out.push('\n');
            }
            if trimmed.starts_with('<')
                && !trimmed.starts_with("</")
                && !trimmed.ends_with("/>")
                && !trimmed.starts_with("<!")
                && !trimmed.to_ascii_lowercase().starts_with("<meta")
                && !trimmed.to_ascii_lowercase().starts_with("<link")
                && !trimmed.to_ascii_lowercase().starts_with("<br")
            {
                indent += 1;
            }
            token.clear();
            in_tag = false;
        }
    }

    let tail = token.trim();
    if !tail.is_empty() {
        out.push_str(&"  ".repeat(indent));
        out.push_str(tail);
        out.push('\n');
    }
    out
}

fn beautify_javascript(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + content.len() / 8);
    let mut indent = 0usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for ch in content.chars() {
        if let Some(quote) = in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => {
                in_string = Some(ch);
                out.push(ch);
            }
            '{' | '[' | '(' => {
                out.push(ch);
                if ch != '(' {
                    indent += 1;
                    newline(&mut out, indent);
                }
            }
            '}' | ']' => {
                indent = indent.saturating_sub(1);
                newline(&mut out, indent);
                out.push(ch);
            }
            ';' => {
                out.push(ch);
                newline(&mut out, indent);
            }
            ',' => {
                out.push(ch);
                newline(&mut out, indent);
            }
            '\n' | '\r' | '\t' => {}
            _ => out.push(ch),
        }
    }
    out
}

fn newline(out: &mut String, indent: usize) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&"  ".repeat(indent));
}
