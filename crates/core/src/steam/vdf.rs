//! Minimal Valve KeyValues (text VDF) reader with span tracking, so a single
//! value can be changed while every other byte of the file stays identical.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub value: Value,
    /// Byte offset where the key token starts.
    pub start: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str { text: String, span: Range<usize> },
    Obj { children: Vec<Node>, open: usize, close: usize },
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str { text, .. } => Some(text),
            Value::Obj { .. } => None,
        }
    }

    pub fn children(&self) -> &[Node] {
        match self {
            Value::Obj { children, .. } => children,
            Value::Str { .. } => &[],
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.children().iter().find(|n| n.key.eq_ignore_ascii_case(key)).map(|n| &n.value)
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Value> {
        keys.iter().try_fold(self, |v, k| v.get(k))
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("VDF parse error at byte {offset}: {message}")]
pub struct VdfError {
    pub offset: usize,
    pub message: &'static str,
}

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String, Range<usize>),
    Open(usize),
    Close(usize),
}

fn tokenize(text: &str) -> Result<Vec<Tok>, VdfError> {
    let b = text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_whitespace() => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                toks.push(Tok::Open(i));
                i += 1;
            }
            b'}' => {
                toks.push(Tok::Close(i));
                i += 1;
            }
            b'[' => {
                // Platform conditional such as [$WIN32]; ignored.
                while i < b.len() && b[i] != b']' {
                    i += 1;
                }
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut s = Vec::new();
                loop {
                    match b.get(i) {
                        None => return Err(VdfError { offset: start, message: "unterminated string" }),
                        Some(b'"') => break,
                        Some(b'\\') => {
                            match b.get(i + 1) {
                                Some(b'n') => s.push(b'\n'),
                                Some(b't') => s.push(b'\t'),
                                Some(b'\\') => s.push(b'\\'),
                                Some(b'"') => s.push(b'"'),
                                Some(&other) => {
                                    s.push(b'\\');
                                    s.push(other);
                                }
                                None => return Err(VdfError { offset: i, message: "dangling escape" }),
                            }
                            i += 2;
                        }
                        Some(&c) => {
                            s.push(c);
                            i += 1;
                        }
                    }
                }
                i += 1;
                toks.push(Tok::Str(String::from_utf8_lossy(&s).into_owned(), start..i));
            }
            _ => {
                let start = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'{' | b'}' | b'"') {
                    i += 1;
                }
                toks.push(Tok::Str(text[start..i].to_string(), start..i));
            }
        }
    }
    Ok(toks)
}

/// Parses a document into a synthetic root object spanning the whole text.
pub fn parse(text: &str) -> Result<Value, VdfError> {
    let toks = tokenize(text)?;
    let mut pos = 0;
    let children = parse_children(&toks, &mut pos, true)?;
    Ok(Value::Obj { children, open: 0, close: text.len() })
}

fn parse_children(toks: &[Tok], pos: &mut usize, top: bool) -> Result<Vec<Node>, VdfError> {
    let mut out = Vec::new();
    loop {
        match toks.get(*pos) {
            None if top => return Ok(out),
            None => return Err(VdfError { offset: usize::MAX, message: "missing closing brace" }),
            Some(Tok::Close(_)) if !top => return Ok(out),
            Some(Tok::Close(at)) => return Err(VdfError { offset: *at, message: "unexpected closing brace" }),
            Some(Tok::Open(at)) => return Err(VdfError { offset: *at, message: "expected a key" }),
            Some(Tok::Str(key, kspan)) => {
                *pos += 1;
                let value = match toks.get(*pos) {
                    Some(Tok::Str(text, span)) => {
                        *pos += 1;
                        Value::Str { text: text.clone(), span: span.clone() }
                    }
                    Some(Tok::Open(open)) => {
                        let open = *open;
                        *pos += 1;
                        let children = parse_children(toks, pos, false)?;
                        let Some(Tok::Close(close)) = toks.get(*pos) else {
                            return Err(VdfError { offset: open, message: "missing closing brace" });
                        };
                        let close = *close;
                        *pos += 1;
                        Value::Obj { children, open, close }
                    }
                    _ => return Err(VdfError { offset: kspan.start, message: "key without value" }),
                };
                out.push(Node { key: key.clone(), value, start: kspan.start });
            }
        }
    }
}

pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Returns the whitespace that starts the line containing `offset`.
pub fn line_indent(text: &str, offset: usize) -> &str {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let rest = &text[line_start..];
    &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_and_escapes() {
        let text = "\"Root\"\n{\n\t\"a\"\t\t\"x \\\"y\\\" C:\\\\z\"\n\t\"Obj\" { \"k\" \"v\" } // c\n\tbare token\n}\n";
        let doc = parse(text).unwrap();
        let root = doc.get("root").unwrap();
        assert_eq!(root.get("a").unwrap().as_str(), Some(r#"x "y" C:\z"#));
        assert_eq!(root.path(&["obj", "K"]).unwrap().as_str(), Some("v"));
        assert_eq!(root.get("bare").unwrap().as_str(), Some("token"));
        if let Value::Str { span, .. } = root.get("a").unwrap() {
            assert_eq!(&text[span.clone()], "\"x \\\"y\\\" C:\\\\z\"");
        }
    }

    #[test]
    fn rejects_broken_files() {
        assert!(parse("\"a\" { \"b\" \"c\"").is_err());
        assert!(parse("\"a\" \"unterminated").is_err());
        assert!(parse("}").is_err());
        assert!(parse("\"a\"").is_err());
    }

    #[test]
    fn escape_round_trips() {
        let v = r#""C:\Program Files\x.exe" --steam id -- %command%"#;
        let text = format!("\"k\" \"{}\"", escape(v));
        assert_eq!(parse(&text).unwrap().get("k").unwrap().as_str(), Some(v));
    }
}
