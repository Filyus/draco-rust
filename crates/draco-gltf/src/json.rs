//! Small, dependency-free JSON DOM used by the lossless glTF model.

use std::ops::{Index, IndexMut};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn parse(input: &[u8]) -> Result<Self, String> {
        let mut parser = Parser { input, pos: 0 };
        let value = parser.value()?;
        parser.space();
        if parser.pos != input.len() {
            return Err("trailing JSON data".into());
        }
        Ok(value)
    }
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
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }
    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(String, Value)>> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(v) => v.parse().ok(),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_object_mut()?
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
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
                            let hex = self
                                .input
                                .get(self.pos..self.pos + 4)
                                .ok_or("short unicode escape")?;
                            self.pos += 4;
                            let text =
                                std::str::from_utf8(hex).map_err(|_| "invalid unicode escape")?;
                            let unit = u16::from_str_radix(text, 16)
                                .map_err(|_| "invalid unicode escape")?;
                            out.push(char::from_u32(unit as u32).ok_or("invalid unicode scalar")?);
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
        while self
            .input
            .get(self.pos)
            .is_some_and(|c| matches!(c, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
        {
            self.pos += 1;
        }
        let text =
            std::str::from_utf8(&self.input[start..self.pos]).map_err(|_| "invalid number")?;
        if text.parse::<f64>().is_err() {
            return Err("invalid number".into());
        }
        Ok(Value::Number(text.into()))
    }
}
