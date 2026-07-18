//! Small, dependency-free JSON DOM used by the lossless glTF model.

use std::ops::{Index, IndexMut};

/// Dependency-free JSON value that preserves number lexemes and object order.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// JSON null.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON number stored as its original lexical representation.
    Number(String),
    /// JSON string.
    String(String),
    /// JSON array.
    Array(Vec<Value>),
    /// JSON object represented as ordered key/value pairs.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Parses one complete JSON value.
    pub fn parse(input: &[u8]) -> Result<Self, String> {
        let mut parser = Parser { input, pos: 0 };
        let value = parser.value()?;
        parser.space();
        if parser.pos != input.len() {
            return Err("trailing JSON data".into());
        }
        Ok(value)
    }
    /// Serializes this value as whitespace-free JSON.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Null => out.extend_from_slice(b"null"),
            Self::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
            Self::Number(v) => out.extend_from_slice(v.as_bytes()),
            Self::String(v) => write_string(out, v),
            Self::Array(values) => {
                out.push(b'[');
                for (i, v) in values.iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    v.write(out);
                }
                out.push(b']');
            }
            Self::Object(values) => {
                out.push(b'{');
                for (i, (k, v)) in values.iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    write_string(out, k);
                    out.push(b':');
                    v.write(out);
                }
                out.push(b'}');
            }
        }
    }
    /// Borrows object entries when this value is an object.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    /// Returns whether this value is an object.
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }
    /// Mutably borrows object entries when this value is an object.
    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(String, Value)>> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    /// Borrows array entries when this value is an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    /// Mutably borrows array entries when this value is an array.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    /// Borrows the string when this value is a string.
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
    /// Parses a non-negative integer without changing its stored lexeme.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(v) => v.parse().ok(),
            _ => None,
        }
    }
    /// Parses a JSON number as `f64` without changing its stored lexeme.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(v) => v.parse().ok(),
            _ => None,
        }
    }
    /// Looks up an object member by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
    /// Mutably looks up an object member by key.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_object_mut()?
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
    /// Constructs an ordered JSON object from key/value entries.
    pub fn object(entries: impl IntoIterator<Item = (impl Into<String>, Value)>) -> Self {
        Self::Object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Self::Number(v.to_string())
    }
}
impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Self::Number(v.to_string())
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
static NULL: Value = Value::Null;
impl Index<&str> for Value {
    type Output = Value;
    fn index(&self, k: &str) -> &Self::Output {
        self.get(k).unwrap_or(&NULL)
    }
}
impl Index<&String> for Value {
    type Output = Value;
    fn index(&self, k: &String) -> &Self::Output {
        self.get(k).unwrap_or(&NULL)
    }
}
impl Index<usize> for Value {
    type Output = Value;
    fn index(&self, i: usize) -> &Self::Output {
        self.as_array().and_then(|v| v.get(i)).unwrap_or(&NULL)
    }
}
impl IndexMut<&str> for Value {
    fn index_mut(&mut self, k: &str) -> &mut Self::Output {
        if !matches!(self, Self::Object(_)) {
            *self = Self::Object(Vec::new());
        }
        let v = self.as_object_mut().unwrap();
        if let Some(i) = v.iter().position(|(name, _)| name == k) {
            &mut v[i].1
        } else {
            v.push((k.into(), Self::Null));
            &mut v.last_mut().unwrap().1
        }
    }
}
impl IndexMut<usize> for Value {
    fn index_mut(&mut self, i: usize) -> &mut Self::Output {
        &mut self.as_array_mut().expect("JSON value is not an array")[i]
    }
}

fn write_string(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    for ch in value.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if c < ' ' => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut b = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
    out.push(b'"');
}
struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}
impl<'a> Parser<'a> {
    fn space(&mut self) {
        while self
            .input
            .get(self.pos)
            .is_some_and(|c| c.is_ascii_whitespace())
        {
            self.pos += 1;
        }
    }
    fn take(&mut self, c: u8) -> bool {
        self.space();
        if self.input.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        self.space();
        match self.input.get(self.pos).copied() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("expected JSON value".into()),
        }
    }
    fn literal(&mut self, s: &[u8], v: Value) -> Result<Value, String> {
        if self.input.get(self.pos..self.pos + s.len()) == Some(s) {
            self.pos += s.len();
            Ok(v)
        } else {
            Err("invalid JSON literal".into())
        }
    }
    fn object(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut o = Vec::new();
        self.space();
        if self.take(b'}') {
            return Ok(Value::Object(o));
        }
        loop {
            self.space();
            if self.input.get(self.pos) != Some(&b'"') {
                return Err("object key is not a string".into());
            }
            let k = self.string()?;
            if !self.take(b':') {
                return Err("missing object colon".into());
            }
            let v = self.value()?;
            o.push((k, v));
            if self.take(b'}') {
                break;
            }
            if !self.take(b',') {
                return Err("missing object comma".into());
            }
        }
        Ok(Value::Object(o))
    }
    fn array(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut a = Vec::new();
        if self.take(b']') {
            return Ok(Value::Array(a));
        }
        loop {
            a.push(self.value()?);
            if self.take(b']') {
                break;
            }
            if !self.take(b',') {
                return Err("missing array comma".into());
            }
        }
        Ok(Value::Array(a))
    }
    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let b = *self.input.get(self.pos).ok_or("unterminated string")?;
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = *self.input.get(self.pos).ok_or("bad escape")?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.unicode_escape()?;
                            let scalar = match first {
                                0xd800..=0xdbff => {
                                    if self.input.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                        return Err("unpaired high surrogate".into());
                                    }
                                    self.pos += 2;
                                    let second = self.unicode_escape()?;
                                    if !(0xdc00..=0xdfff).contains(&second) {
                                        return Err("invalid low surrogate".into());
                                    }
                                    0x1_0000
                                        + (u32::from(first - 0xd800) << 10)
                                        + u32::from(second - 0xdc00)
                                }
                                0xdc00..=0xdfff => return Err("unpaired low surrogate".into()),
                                value => u32::from(value),
                            };
                            out.push(char::from_u32(scalar).ok_or("invalid unicode scalar")?);
                        }
                        _ => return Err("invalid escape".into()),
                    }
                }
                0..=0x1f => return Err("control character in string".into()),
                _ => {
                    let rest = &self.input[self.pos - 1..];
                    let ch = std::str::from_utf8(rest)
                        .map_err(|_| "invalid utf8")?
                        .chars()
                        .next()
                        .unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8() - 1;
                }
            }
        }
    }
    fn number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.input.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.input.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while self
                    .input
                    .get(self.pos)
                    .is_some_and(|byte| byte.is_ascii_digit())
                {
                    self.pos += 1;
                }
            }
            _ => return Err("invalid number".into()),
        }
        if self.input.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !self
                .input
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                return Err("invalid number fraction".into());
            }
            while self
                .input
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                self.pos += 1;
            }
        }
        if self
            .input
            .get(self.pos)
            .is_some_and(|byte| matches!(byte, b'e' | b'E'))
        {
            self.pos += 1;
            if self
                .input
                .get(self.pos)
                .is_some_and(|byte| matches!(byte, b'+' | b'-'))
            {
                self.pos += 1;
            }
            if !self
                .input
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                return Err("invalid number exponent".into());
            }
            while self
                .input
                .get(self.pos)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                self.pos += 1;
            }
        }
        let text =
            std::str::from_utf8(&self.input[start..self.pos]).map_err(|_| "invalid number")?;
        Ok(Value::Number(text.into()))
    }

    fn unicode_escape(&mut self) -> Result<u16, String> {
        let hex = self
            .input
            .get(self.pos..self.pos + 4)
            .ok_or("short unicode escape")?;
        self.pos += 4;
        let text = std::str::from_utf8(hex).map_err(|_| "invalid unicode escape")?;
        u16::from_str_radix(text, 16).map_err(|_| "invalid unicode escape".into())
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    #[test]
    fn parses_unicode_surrogate_pairs() {
        assert_eq!(
            Value::parse(br#""\ud83d\ude80""#).unwrap(),
            Value::String("🚀".into())
        );
        assert!(Value::parse(br#""\ud83d""#).is_err());
        assert!(Value::parse(br#""\ude80""#).is_err());
    }

    #[test]
    fn enforces_json_number_grammar_without_float_range_limits() {
        assert!(Value::parse(b"123456789012345678901234567890e999999").is_ok());
        for invalid in [
            b"01".as_slice(),
            b"1.".as_slice(),
            b"1e".as_slice(),
            b"-".as_slice(),
        ] {
            assert!(
                Value::parse(invalid).is_err(),
                "{invalid:?} should be invalid"
            );
        }
    }
}
