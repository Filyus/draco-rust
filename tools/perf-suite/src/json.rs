//! The flat JSON objects the harnesses write, one a line, and nothing more.
//!
//! A row is an object of strings, numbers, booleans and nulls. Nesting never
//! occurs, so this reads that shape and rejects anything else, rather than
//! pulling a general JSON library into a tool with no other dependency.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Text(String),
    Number(f64),
    Flag(bool),
    Null,
}

pub type Row = BTreeMap<String, Json>;

/// Typed reads of a row's fields. A missing or null number reads as NaN, so
/// it prints as such in a table instead of passing for zero.
pub trait Fields {
    fn text(&self, key: &str) -> String;
    fn number(&self, key: &str) -> f64;
    fn flag(&self, key: &str) -> Option<bool>;
}

impl Fields for Row {
    fn text(&self, key: &str) -> String {
        match self.get(key) {
            Some(Json::Text(text)) => text.clone(),
            Some(Json::Number(number)) => number.to_string(),
            Some(Json::Flag(flag)) => flag.to_string(),
            Some(Json::Null) | None => String::new(),
        }
    }

    fn number(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(Json::Number(number)) => *number,
            _ => f64::NAN,
        }
    }

    fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Json::Flag(flag)) => Some(*flag),
            _ => None,
        }
    }
}

/// `text` as a JSON string literal.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An object of string fields, written in the order given.
pub fn object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{}:{}", quote(key), quote(value)))
        .collect();
    format!("{{{}}}", body.join(","))
}

pub fn parse_line(line: &str) -> Result<Row, String> {
    let mut parser = Parser {
        bytes: line.as_bytes(),
        at: 0,
    };
    let row = parser.object()?;
    parser.space();
    if parser.at != parser.bytes.len() {
        return Err(parser.error("trailing characters"));
    }
    Ok(row)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.at)
    }

    fn space(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.space();
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", byte as char)))
        }
    }

    fn object(&mut self) -> Result<Row, String> {
        self.expect(b'{')?;
        let mut row = Row::new();
        self.space();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(row);
        }
        loop {
            self.space();
            let key = self.string()?;
            self.expect(b':')?;
            let value = self.value()?;
            row.insert(key, value);
            self.space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(row);
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.space();
        let rest = &self.bytes[self.at..];
        for (word, value) in [
            ("true", Json::Flag(true)),
            ("false", Json::Flag(false)),
            ("null", Json::Null),
        ] {
            if rest.starts_with(word.as_bytes()) {
                self.at += word.len();
                return Ok(value);
            }
        }
        if rest.first() == Some(&b'"') {
            return self.string().map(Json::Text);
        }
        let len = rest
            .iter()
            .position(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
            .unwrap_or(rest.len());
        let number = std::str::from_utf8(&rest[..len])
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| self.error("expected a value"))?;
        self.at += len;
        Ok(Json::Number(number))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return Err(self.error("expected a string"));
        }
        self.at += 1;
        let mut out = String::new();
        loop {
            // A run up to the next quote or backslash is whole UTF-8: neither
            // byte occurs inside a multi-byte sequence.
            let start = self.at;
            while self
                .bytes
                .get(self.at)
                .is_some_and(|&b| b != b'"' && b != b'\\')
            {
                self.at += 1;
            }
            let run = std::str::from_utf8(&self.bytes[start..self.at])
                .map_err(|_| self.error("invalid UTF-8"))?;
            out.push_str(run);
            match self.bytes.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    let escaped = *self
                        .bytes
                        .get(self.at + 1)
                        .ok_or_else(|| self.error("unfinished escape"))?;
                    self.at += 2;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let code = self
                                .bytes
                                .get(self.at..self.at + 4)
                                .and_then(|hex| std::str::from_utf8(hex).ok())
                                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                                .ok_or_else(|| self.error("bad \\u escape"))?;
                            self.at += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                _ => return Err(self.error("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_field() {
        let row = parse_line(
            r#"{"table":"grid_encode","model":"a \"b\"\\c\u00e9","speed":5,"cpp_us":-1.5e3,"ok":true,"no":false,"gone":null}"#,
        )
        .unwrap();
        assert_eq!(row.text("table"), "grid_encode");
        assert_eq!(row.text("model"), "a \"b\"\\c\u{e9}");
        assert_eq!(row.number("speed"), 5.0);
        assert_eq!(row.number("cpp_us"), -1500.0);
        assert_eq!(row.flag("ok"), Some(true));
        assert_eq!(row.flag("no"), Some(false));
        assert!(row.number("gone").is_nan());
        assert!(row.number("missing").is_nan());
    }

    #[test]
    fn what_it_writes_it_reads_back() {
        let line = object(&[("key", "line\nbreak \"quoted\" \u{1} \u{e9}".to_owned())]);
        let row = parse_line(&line).unwrap();
        assert_eq!(row.text("key"), "line\nbreak \"quoted\" \u{1} \u{e9}");
    }

    #[test]
    fn rejects_what_is_not_one_flat_object() {
        for line in [
            "",
            "{",
            r#"{"a":1"#,
            r#"{"a":1}x"#,
            r#"{"a":[1]}"#,
            r#"{"a":"open}"#,
            r#"{a:1}"#,
        ] {
            assert!(parse_line(line).is_err(), "{line:?} parsed");
        }
    }
}
