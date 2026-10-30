//! ASCII FBX (7.x) records, read into the same tree as the binary layout so
//! everything after the records is shared.
//!
//! Written from the publicly described text form: `;` comments to the end of
//! a line, then records written `Name: value, value, ... { children }`. A
//! value is a number, a quoted string (a quote inside one written `&quot;`)
//! or a bare letter (`T`/`Y` true, `F`/`N` false); a value list may go on
//! over several lines after a comma. An array is written `Name: *count { a:
//! v, v, ... }`. Objects are named `Class::Name` (the binary form's
//! `Name\0\x01Class`), which the scene reader already accepts, and an embedded
//! file (`Content`) is base64 text, decoded here to the raw bytes the binary
//! form stores. The version is `FBXHeaderExtension`'s `FBXVersion`.
//!
//! The text is untrusted: nesting is depth-limited and arrays are checked
//! against the count they declare and the binary reader's size limit.

use anyhow::{Context, Result, bail, ensure};

use super::{MAX_ARRAY, MAX_DEPTH, Node, Value};

/// Whether `bytes` starts like an ASCII FBX (comments, then records).
pub(crate) fn looks_ascii(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let text = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let starts = text
        .iter()
        .find(|c| !c.is_ascii_whitespace())
        .is_some_and(|&c| c == b';' || c.is_ascii_alphabetic());
    starts
        && (text.windows(18).any(|w| w == b"FBXHeaderExtension")
            || text.windows(5).any(|w| w == b"; FBX"))
}

/// The top-level records of an ASCII FBX file, and its version.
pub(crate) fn parse(bytes: &[u8]) -> Result<(u32, Vec<Node>)> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).context("the ASCII FBX isn't UTF-8 text")?;
    let mut p = Parser {
        s: text.as_bytes(),
        at: 0,
        line: 1,
    };
    let top = p.nodes(0, false)?;
    let version = top
        .iter()
        .find(|n| n.name == "FBXHeaderExtension")
        .and_then(|h| h.value("FBXVersion"))
        .and_then(Value::as_i64)
        .or_else(|| header_comment_version(text))
        .context("the ASCII FBX has no FBXVersion")?;
    ensure!(
        (7000..8000).contains(&version),
        "ASCII FBX version {version} isn't supported (7.x is)"
    );
    Ok((version as u32, top))
}

/// `; FBX 7.4.0 project file` → 7400.
fn header_comment_version(text: &str) -> Option<i64> {
    let line = text.lines().find(|l| l.trim_start().starts_with(';'))?;
    let rest = line.split("FBX").nth(1)?.split_whitespace().next()?;
    let mut parts = rest.split('.').map(|p| p.parse::<i64>().ok());
    let (a, b, c) = (parts.next()??, parts.next()??, parts.next().flatten());
    Some(a * 1000 + b * 100 + c.unwrap_or(0) * 10)
}

/// A property as written, before arrays are put together.
enum Item {
    Value(Value),
    /// `*count`: the array's values follow in the `a` child.
    Count(usize),
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    line: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.at += 1;
        if c == b'\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn fail<T>(&self, what: &str) -> Result<T> {
        bail!("ASCII FBX line {}: {what}", self.line)
    }

    /// Skip spaces and comments; newlines too when `lines`.
    fn skip(&mut self, lines: bool) {
        while let Some(c) = self.peek() {
            match c {
                b';' => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.bump();
                    }
                }
                b'\n' if !lines => return,
                c if c.is_ascii_whitespace() => {
                    self.bump();
                }
                _ => return,
            }
        }
    }

    fn word(&mut self) -> &str {
        let start = self.at;
        while self.peek().is_some_and(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'+' | b'.' | b'|' | b'#')
        }) {
            self.bump();
        }
        // Only ASCII bytes were taken.
        std::str::from_utf8(&self.s[start..self.at]).unwrap_or("")
    }

    /// Records until the end of the text, or until a `}` when `nested`.
    fn nodes(&mut self, depth: usize, nested: bool) -> Result<Vec<Node>> {
        if depth >= MAX_DEPTH {
            return self.fail("records nest too deeply");
        }
        let mut out = Vec::new();
        loop {
            self.skip(true);
            match self.peek() {
                None if nested => return self.fail("a `{` is never closed"),
                None => return Ok(out),
                Some(b'}') if nested => {
                    self.bump();
                    return Ok(out);
                }
                Some(b'}') => return self.fail("a `}` closes nothing"),
                _ => out.push(self.node(depth)?),
            }
        }
    }

    fn node(&mut self, depth: usize) -> Result<Node> {
        let name = self.word().to_string();
        if name.is_empty() {
            return self.fail("expected a record name");
        }
        self.skip(false);
        if self.bump() != Some(b':') {
            return self.fail(&format!("expected `:` after {name}"));
        }
        let mut items = Vec::new();
        let mut children = Vec::new();
        // After a comma the list goes on, over newlines too.
        let mut more = false;
        loop {
            self.skip(more);
            match self.peek() {
                None | Some(b'\n') | Some(b'}') => break,
                Some(b'{') => {
                    self.bump();
                    children = self.nodes(depth + 1, true)?;
                    break;
                }
                Some(b',') => {
                    self.bump();
                    more = true;
                    continue;
                }
                Some(b'"') => {
                    self.bump();
                    items.push(Item::Value(Value::Str(self.string()?)));
                }
                Some(b'*') => {
                    self.bump();
                    let n = self.word();
                    let Ok(n) = n.parse::<usize>() else {
                        return self.fail(&format!("{name}: a bad array count"));
                    };
                    items.push(Item::Count(n));
                }
                Some(_) => {
                    let w = self.word().to_string();
                    if w.is_empty() {
                        return self.fail(&format!("{name}: an unexpected character"));
                    }
                    items.push(Item::Value(bare(&w)));
                }
            }
            more = false;
            self.skip(false);
            match self.peek() {
                Some(b',') => {
                    self.bump();
                    more = true;
                }
                None | Some(b'\n') | Some(b'{') | Some(b'}') => {}
                Some(_) => return self.fail(&format!("{name}: expected `,` between values")),
            }
        }
        let mut props = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Item::Value(v) => props.push(v),
                Item::Count(n) => {
                    let values = children
                        .iter()
                        .position(|c| c.name == "a")
                        .map(|i| children.remove(i).props)
                        .unwrap_or_default();
                    props.push(
                        array(&name, n, values)
                            .with_context(|| format!("ASCII FBX line {}: {name}", self.line))?,
                    );
                }
            }
        }
        if name == "Content" {
            // An embedded file: base64 text, maybe in several strings.
            let mut b64 = String::new();
            for p in &props {
                if let Value::Str(s) = p {
                    b64.push_str(s);
                }
            }
            if !b64.is_empty() {
                props = vec![Value::Raw(base64(&b64).with_context(|| {
                    format!("ASCII FBX line {}: Content isn't base64", self.line)
                })?)];
            }
        }
        Ok(Node {
            name,
            props,
            children,
        })
    }

    /// A quoted string, after its opening quote.
    fn string(&mut self) -> Result<String> {
        let start = self.at;
        loop {
            match self.bump() {
                None => return self.fail("a string is never closed"),
                Some(b'"') => break,
                _ => {}
            }
        }
        let raw = std::str::from_utf8(&self.s[start..self.at - 1]).unwrap_or("");
        Ok(if raw.contains('&') {
            raw.replace("&quot;", "\"")
                .replace("&lf;", "\n")
                .replace("&cr;", "\r")
        } else {
            raw.to_string()
        })
    }
}

/// A bare value: a number, or a letter for true or false.
fn bare(w: &str) -> Value {
    if let Ok(i) = w.parse::<i64>() {
        return Value::I64(i);
    }
    if let Ok(f) = w.parse::<f64>() {
        return Value::F64(f);
    }
    match w {
        "T" | "Y" => Value::Bool(true),
        "F" | "N" => Value::Bool(false),
        _ => Value::Str(w.to_string()),
    }
}

/// An array's values: integers if every one is, doubles otherwise.
fn array(name: &str, count: usize, values: Vec<Value>) -> Result<Value> {
    ensure!(
        count.checked_mul(8).is_some_and(|l| l <= MAX_ARRAY),
        "an array is too large"
    );
    ensure!(
        values.len() == count,
        "the array has {} values but declares {count}",
        values.len()
    );
    if values.iter().all(|v| matches!(v, Value::I64(_))) {
        return Ok(Value::I64s(
            values.iter().filter_map(Value::as_i64).collect(),
        ));
    }
    if values.iter().all(|v| matches!(v, Value::Bool(_))) {
        return Ok(Value::Bools(
            values
                .iter()
                .map(|v| matches!(v, Value::Bool(true)))
                .collect(),
        ));
    }
    values
        .iter()
        .map(|v| match v {
            Value::I64(_) | Value::F64(_) => v.as_f64(),
            _ => None,
        })
        .collect::<Option<Vec<f64>>>()
        .map(Value::F64s)
        .with_context(|| format!("{name}: an array value isn't a number"))
}

/// Standard base64 (padding and whitespace allowed).
fn base64(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => bail!("not base64"),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_records_values_arrays_and_comments() {
        let text = "; FBX 7.4.0 project file\n\
            ; a comment\n\
            FBXHeaderExtension:  {\n\
            \tFBXVersion: 7400 ; trailing comment\n\
            }\n\
            Objects:  {\n\
            \tGeometry: 140, \"Geometry::Cube\", \"Mesh\" {\n\
            \t\tVertices: *6 {\n\
            \t\t\ta: 1,-2.5,3e-2,\n\
            4,5,6\n\
            \t\t}\n\
            \t\tPolygonVertexIndex: *3 {\n\
            \t\t\ta: 0,1,-3\n\
            \t\t}\n\
            \t\tShading: T\n\
            \t\tCulling: \"Culling&quot;Off\"\n\
            \t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",0,1.5,\n\
            \t\t\t2\n\
            \t}\n\
            \tVideo: 7, \"Video::v\", \"Clip\" {\n\
            \t\tContent: , \"aGl2\",\n\
            \t\t\"ZQ==\"\n\
            \t}\n\
            }\n";
        assert!(looks_ascii(text.as_bytes()));
        // Cut anywhere or with a changed byte, it reads or fails, never panics.
        for at in 0..text.len() {
            let _ = parse(&text.as_bytes()[..at]);
            let mut b = text.as_bytes().to_vec();
            b[at] = b"{}\",*:;\n\xff9"[at % 10];
            let _ = parse(&b);
        }
        let (version, top) = parse(text.as_bytes()).unwrap();
        assert_eq!(version, 7400);
        let geom = &top[1].children[0];
        assert_eq!(geom.id(), Some(140));
        assert_eq!(geom.object_name(), "Cube");
        assert_eq!(geom.class(), "Mesh");
        assert_eq!(
            geom.value("Vertices"),
            Some(&Value::F64s(vec![1.0, -2.5, 0.03, 4.0, 5.0, 6.0]))
        );
        assert_eq!(
            geom.value("PolygonVertexIndex"),
            Some(&Value::I64s(vec![0, 1, -3]))
        );
        assert_eq!(geom.value("Shading"), Some(&Value::Bool(true)));
        assert_eq!(
            geom.value("Culling"),
            Some(&Value::Str("Culling\"Off".into()))
        );
        let p = geom.child("P").unwrap();
        assert_eq!(p.props.len(), 7);
        assert_eq!(p.props[6], Value::I64(2));
        let video = &top[1].children[1];
        assert_eq!(video.value("Content"), Some(&Value::Raw(b"hive".to_vec())));
    }

    #[test]
    fn refuses_bad_text() {
        for bad in [
            "FBXHeaderExtension: {\n FBXVersion: 7400\n",
            "FBXHeaderExtension: {\n FBXVersion: 6100\n}\n",
            "FBXHeaderExtension: {\n FBXVersion: 7400\n}\nA: *3 {\n a: 1,2\n}\n",
            "FBXHeaderExtension: {\n FBXVersion: 7400\n}\nA: *99999999999999 {\n}\n",
            "FBXHeaderExtension: {\n FBXVersion: 7400\n}\nA: 1 2\n",
            "FBXHeaderExtension: {\n FBXVersion: 7400\n}\nA: \"open\n",
            "FBXHeaderExtension: {\n FBXVersion: 7400\n}\n}\n",
            "FBXHeaderExtension: {\n FBXVersion: 7400\n}\nContent: \"@@@\"\n",
        ] {
            assert!(parse(bad.as_bytes()).is_err(), "{bad}");
        }
        let deep = format!(
            "FBXHeaderExtension: {{\n FBXVersion: 7400\n}}\n{}",
            "A: {\n".repeat(MAX_DEPTH + 2)
        );
        assert!(parse(deep.as_bytes()).is_err());
        assert_eq!(
            header_comment_version("; FBX 7.5.0 project file"),
            Some(7500)
        );
    }
}
