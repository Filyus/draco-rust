//! Small, dependency-free JSON DOM used by the lossless glTF model.

use std::fmt;
use std::mem;
use std::ops::{Index, IndexMut};
use std::slice;

/// Dependency-free JSON value that preserves number lexemes and object order.
///
/// The type is recursive, so every operation that walks a whole tree --
/// parsing, serializing, cloning, comparing and dropping -- carries the input's
/// nesting in an explicit heap stack rather than in call frames. Nesting is
/// therefore bounded by memory proportional to the document, not by the thread
/// stack, and no operation on a value that was parsed successfully can overflow
/// it afterwards.
///
/// That holds only as far as the code that consumes a value: since no depth is
/// refused at parse time, a walk over a parsed tree must carry its own stack
/// too. A recursive one turns a hostile document into a stack overflow, which
/// is the whole failure this design removes.
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
        // A container whose opening bracket is already written: what is left to
        // serialize, and whether a separator is owed before the next entry.
        enum Frame<'a> {
            Array(slice::Iter<'a, Value>, bool),
            Object(slice::Iter<'a, (String, Value)>, bool),
        }
        let mut stack: Vec<Frame<'_>> = Vec::new();
        let mut pending = Some(self);
        loop {
            if let Some(value) = pending.take() {
                match value {
                    Self::Null => out.extend_from_slice(b"null"),
                    Self::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
                    Self::Number(v) => out.extend_from_slice(v.as_bytes()),
                    Self::String(v) => write_string(out, v),
                    Self::Array(values) => {
                        out.push(b'[');
                        stack.push(Frame::Array(values.iter(), false));
                    }
                    Self::Object(values) => {
                        out.push(b'{');
                        stack.push(Frame::Object(values.iter(), false));
                    }
                }
            }
            match stack.last_mut() {
                None => return,
                Some(Frame::Array(rest, separate)) => match rest.next() {
                    Some(value) => {
                        if mem::replace(separate, true) {
                            out.push(b',');
                        }
                        pending = Some(value);
                    }
                    None => {
                        out.push(b']');
                        stack.pop();
                    }
                },
                Some(Frame::Object(rest, separate)) => match rest.next() {
                    Some((key, value)) => {
                        if mem::replace(separate, true) {
                            out.push(b',');
                        }
                        write_string(out, key);
                        out.push(b':');
                        pending = Some(value);
                    }
                    None => {
                        out.push(b'}');
                        stack.pop();
                    }
                },
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
    /// Takes the array entries out of this value.
    ///
    /// `Value` frees itself iteratively, and a type with a destructor cannot
    /// have a variant's payload moved out of it by pattern matching, so the
    /// three `into_` methods are how a caller takes ownership of one.
    pub fn into_array(mut self) -> Option<Vec<Value>> {
        self.as_array_mut().map(mem::take)
    }
    /// Takes the object entries out of this value.
    pub fn into_object(mut self) -> Option<Vec<(String, Value)>> {
        self.as_object_mut().map(mem::take)
    }
    /// Takes the string out of this value.
    pub fn into_string(mut self) -> Option<String> {
        match &mut self {
            Self::String(v) => Some(mem::take(v)),
            _ => None,
        }
    }
}
impl Clone for Value {
    fn clone(&self) -> Self {
        // A container being rebuilt: what is left to copy from the source, and
        // what has been copied so far. Object frames also carry the key whose
        // value is currently being cloned.
        enum Frame<'a> {
            Array(slice::Iter<'a, Value>, Vec<Value>),
            Object(
                slice::Iter<'a, (String, Value)>,
                Vec<(String, Value)>,
                &'a str,
            ),
        }
        let mut stack: Vec<Frame<'_>> = Vec::new();
        let mut source = Some(self);
        // The subtree finished most recently, waiting to be stored in its parent.
        let mut done: Option<Value> = None;
        loop {
            if let Some(value) = source.take() {
                match value {
                    Self::Null => done = Some(Self::Null),
                    Self::Bool(v) => done = Some(Self::Bool(*v)),
                    Self::Number(v) => done = Some(Self::Number(v.clone())),
                    Self::String(v) => done = Some(Self::String(v.clone())),
                    Self::Array(values) => {
                        stack.push(Frame::Array(
                            values.iter(),
                            Vec::with_capacity(values.len()),
                        ));
                    }
                    Self::Object(values) => {
                        stack.push(Frame::Object(
                            values.iter(),
                            Vec::with_capacity(values.len()),
                            "",
                        ));
                    }
                }
            }
            let finished = match stack.last_mut() {
                None => return done.expect("the root value is cloned before the stack empties"),
                Some(Frame::Array(rest, out)) => {
                    if let Some(value) = done.take() {
                        out.push(value);
                    }
                    match rest.next() {
                        Some(next) => {
                            source = Some(next);
                            false
                        }
                        None => true,
                    }
                }
                Some(Frame::Object(rest, out, key)) => {
                    if let Some(value) = done.take() {
                        out.push(((*key).to_owned(), value));
                    }
                    match rest.next() {
                        Some((next_key, next)) => {
                            *key = next_key;
                            source = Some(next);
                            false
                        }
                        None => true,
                    }
                }
            };
            if finished {
                done = Some(
                    match stack.pop().expect("a frame was just observed on the stack") {
                        Frame::Array(_, out) => Self::Array(out),
                        Frame::Object(_, out, _) => Self::Object(out),
                    },
                );
            }
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        let mut work = vec![(self, other)];
        while let Some(pair) = work.pop() {
            match pair {
                (Self::Null, Self::Null) => {}
                (Self::Bool(a), Self::Bool(b)) if a == b => {}
                (Self::Number(a), Self::Number(b)) | (Self::String(a), Self::String(b))
                    if a == b => {}
                (Self::Array(a), Self::Array(b)) if a.len() == b.len() => {
                    work.extend(a.iter().zip(b));
                }
                (Self::Object(a), Self::Object(b)) if a.len() == b.len() => {
                    for ((key, a), (other_key, b)) in a.iter().zip(b) {
                        if key != other_key {
                            return false;
                        }
                        work.push((a, b));
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

impl fmt::Debug for Value {
    /// Renders the value as JSON text. The derived form would recurse through
    /// containers, so a deep value could only be formatted by overflowing the
    /// stack.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.to_vec()))
    }
}

impl Drop for Value {
    /// Frees the tree breadth-first through a worklist.
    ///
    /// Every value reaches its own `drop` with its children already moved onto
    /// the worklist, so the implicit drop of each child bottoms out immediately
    /// instead of descending another level.
    fn drop(&mut self) {
        let mut work = Vec::new();
        Self::orphan_children(self, &mut work);
        while let Some(mut value) = work.pop() {
            Self::orphan_children(&mut value, &mut work);
        }
    }
}

impl Value {
    /// Moves a container's children onto `work`, leaving the value empty.
    fn orphan_children(value: &mut Self, work: &mut Vec<Self>) {
        match value {
            Self::Array(values) => work.append(values),
            Self::Object(values) => work.extend(mem::take(values).into_iter().map(|(_, v)| v)),
            _ => {}
        }
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
/// A container the parser has opened but not yet closed.
///
/// Object frames also hold the key whose value is being parsed, so that the key
/// and its value only become a pair once the value is complete.
enum Frame {
    Array(Vec<Value>),
    Object(Vec<(String, Value)>, String),
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
    /// Parses one complete value, holding open containers on the heap.
    ///
    /// Nesting costs one frame per level instead of a call frame, so the only
    /// bound on depth is the memory the document itself pays for: a level
    /// cannot be opened without spending an input byte on its bracket. The
    /// worst shape for this, an input that is nothing but brackets, peaks near
    /// 90 bytes of heap per input byte, against roughly 17 for a document that
    /// carries actual content.
    fn value(&mut self) -> Result<Value, String> {
        let mut stack: Vec<Frame> = Vec::new();
        // The value finished most recently, waiting to be stored in the
        // container that encloses it.
        let mut done;
        'value: loop {
            self.space();
            match self.input.get(self.pos).copied() {
                Some(b'{') => {
                    self.pos += 1;
                    if self.take(b'}') {
                        done = Value::Object(Vec::new());
                    } else {
                        let key = self.object_key()?;
                        stack.push(Frame::Object(Vec::new(), key));
                        continue 'value;
                    }
                }
                Some(b'[') => {
                    self.pos += 1;
                    if self.take(b']') {
                        done = Value::Array(Vec::new());
                    } else {
                        stack.push(Frame::Array(Vec::new()));
                        continue 'value;
                    }
                }
                Some(b'"') => done = Value::String(self.string()?),
                Some(b't') => done = self.literal(b"true", Value::Bool(true))?,
                Some(b'f') => done = self.literal(b"false", Value::Bool(false))?,
                Some(b'n') => done = self.literal(b"null", Value::Null)?,
                Some(b'-' | b'0'..=b'9') => done = self.number()?,
                _ => return Err("expected JSON value".into()),
            }
            // Store the finished value, then close every container that ends
            // here; each one becomes a finished value for the level above it.
            loop {
                let closed = match stack.last_mut() {
                    None => return Ok(done),
                    Some(Frame::Array(values)) => {
                        values.push(done);
                        if self.take(b']') {
                            true
                        } else if self.take(b',') {
                            false
                        } else {
                            return Err("missing array comma".into());
                        }
                    }
                    Some(Frame::Object(values, key)) => {
                        values.push((mem::take(key), done));
                        if self.take(b'}') {
                            true
                        } else if self.take(b',') {
                            false
                        } else {
                            return Err("missing object comma".into());
                        }
                    }
                };
                if !closed {
                    if let Some(Frame::Object(_, key)) = stack.last_mut() {
                        *key = self.object_key()?;
                    }
                    continue 'value;
                }
                done = match stack.pop().expect("a frame was just observed on the stack") {
                    Frame::Array(values) => Value::Array(values),
                    Frame::Object(values, _) => Value::Object(values),
                };
            }
        }
    }
    /// Consumes one object key and the colon that must follow it.
    fn object_key(&mut self) -> Result<String, String> {
        self.space();
        if self.input.get(self.pos) != Some(&b'"') {
            return Err("object key is not a string".into());
        }
        let key = self.string()?;
        if !self.take(b':') {
            return Err("missing object colon".into());
        }
        Ok(key)
    }
    fn literal(&mut self, s: &[u8], v: Value) -> Result<Value, String> {
        if self.input.get(self.pos..self.pos + s.len()) == Some(s) {
            self.pos += s.len();
            Ok(v)
        } else {
            Err("invalid JSON literal".into())
        }
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
                0x20..=0x7f => out.push(char::from(b)),
                _ => {
                    // Decode exactly one scalar from the lead byte. Validating
                    // the whole remaining input here would make parsing
                    // quadratic in document size.
                    let width = match b {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => return Err("invalid utf8".into()),
                    };
                    let start = self.pos - 1;
                    let encoded = self.input.get(start..start + width).ok_or("invalid utf8")?;
                    let ch = std::str::from_utf8(encoded)
                        .map_err(|_| "invalid utf8")?
                        .chars()
                        .next()
                        .ok_or("invalid utf8")?;
                    out.push(ch);
                    self.pos = start + width;
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
    use super::{mem, Value};

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
    fn parses_raw_multibyte_scalars_and_rejects_malformed_bytes() {
        let source = "\"aé\u{20ac}\u{1f680}\"";
        assert_eq!(
            Value::parse(source.as_bytes()).unwrap(),
            Value::String("aé\u{20ac}\u{1f680}".into())
        );
        for invalid in [
            b"\"\x80\"".as_slice(),             // bare continuation byte
            b"\"\xc0\xaf\"".as_slice(),         // overlong encoding
            b"\"\xed\xa0\x80\"".as_slice(),     // UTF-16 surrogate
            b"\"\xf5\x80\x80\x80\"".as_slice(), // beyond U+10FFFF
            b"\"\xe2\x82\"".as_slice(),         // truncated sequence
        ] {
            assert!(
                Value::parse(invalid).is_err(),
                "{invalid:?} should be invalid"
            );
        }
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

    /// Deep enough that any per-level call frame would overflow the 2 MiB stack
    /// a spawned thread gets by default.
    const DEEP: usize = 200_000;

    /// Deterministic pseudo-random byte source, so a failure is reproducible
    /// from the seed printed with it.
    fn random(seed: u64) -> impl FnMut() -> u64 {
        let mut state = seed | 1;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        }
    }

    #[test]
    fn random_documents_survive_a_parse_serialize_parse_cycle() {
        // Token soup, mostly invalid: parse errors must stay errors.
        let tokens = [
            "{", "}", "[", "]", ",", ":", "\"a\"", "1", "-2.5e3", "true", "null", " ", "01", "\"",
            "\\",
        ];
        // Balanced nesting, always valid: the writer sees shapes no fixture has.
        let mut accepted = 0;
        for seed in 0..4_000u64 {
            let mut next = random(seed);
            let mut soup = String::new();
            for _ in 0..next() % 200 {
                soup.push_str(tokens[(next() % tokens.len() as u64) as usize]);
            }

            let scalars = ["1", "-2.5e3", "true", "false", "null", r#""s""#, "[]", "{}"];
            let mut structured = String::new();
            // Each frame is the bracket that closes it and whether it already
            // holds an entry, which is what decides the separator.
            let mut open: Vec<(char, bool)> = Vec::new();
            structured.push('[');
            open.push((']', false));
            for _ in 0..next() % 200 {
                if open.is_empty() {
                    break;
                }
                let action = next() % 4;
                if action == 3 || open.len() >= 60 {
                    structured.push(open.pop().expect("a container is open").0);
                    continue;
                }
                let (close, filled) = open.last_mut().expect("a container is open");
                let in_object = *close == '}';
                if mem::replace(filled, true) {
                    structured.push(',');
                }
                if in_object {
                    structured.push_str(r#""k":"#);
                }
                match action {
                    0 => {
                        structured.push('[');
                        open.push((']', false));
                    }
                    1 => {
                        structured.push('{');
                        open.push(('}', false));
                    }
                    _ => structured.push_str(scalars[(next() % scalars.len() as u64) as usize]),
                }
            }
            while let Some((close, _)) = open.pop() {
                structured.push(close);
            }

            for source in [soup, structured] {
                let Ok(value) = Value::parse(source.as_bytes()) else {
                    continue;
                };
                accepted += 1;
                let text = value.to_vec();
                let reparsed = Value::parse(&text)
                    .unwrap_or_else(|e| panic!("seed {seed} serialized to unparsable JSON: {e}"));
                assert_eq!(reparsed, value, "seed {seed} changed across a round trip");
                assert_eq!(reparsed.to_vec(), text, "seed {seed} serializes unstably");
                assert_eq!(value.clone(), value, "seed {seed} clones unequal");
            }
        }
        // A generator that stopped producing parsable documents would make the
        // round trip vacuous.
        assert!(accepted > 3_000, "only {accepted} documents parsed");
    }

    #[test]
    fn equality_distinguishes_variants_lexemes_and_member_order() {
        let parse = |text: &str| Value::parse(text.as_bytes()).unwrap();
        assert_eq!(
            parse(r#"{"a":[1,{"b":null}]}"#),
            parse(r#"{"a":[1,{"b":null}]}"#)
        );
        // A shared payload across two variants is not equality.
        assert_ne!(parse("1"), parse(r#""1""#));
        assert_ne!(parse("true"), parse(r#""true""#));
        // Lexemes are preserved, so equal quantities can still differ.
        assert_ne!(parse("1"), parse("1.0"));
        // Objects are ordered pairs, not maps.
        assert_ne!(parse(r#"{"a":1,"b":2}"#), parse(r#"{"b":2,"a":1}"#));
        assert_ne!(parse(r#"{"a":1}"#), parse(r#"{"b":1}"#));
        // Length is compared before the elements are.
        assert_ne!(parse("[1,2]"), parse("[1,2,3]"));
        assert_ne!(parse("[1,2]"), parse("[1,3]"));
        assert_ne!(parse("[]"), parse("{}"));
        assert_ne!(parse("null"), parse("[]"));
    }

    #[test]
    fn owned_accessors_take_the_payload_of_their_own_variant_only() {
        let value = Value::parse(br#"[1,"s"]"#).unwrap();
        assert!(value.clone().into_object().is_none());
        assert!(value.clone().into_string().is_none());
        let items = value.into_array().expect("an array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].clone().into_string().as_deref(), Some("s"));

        let object = Value::parse(br#"{"a":null}"#).unwrap();
        assert!(object.clone().into_array().is_none());
        let entries = object.into_object().expect("an object");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "a");
    }

    #[test]
    fn clone_reproduces_every_variant_of_a_mixed_tree() {
        let value = Value::parse(
            br#"{"a":[1,-2.5e3,"s",true,false,null,[],{},[{"b":[[1]]}]],"c":{"d":{}}}"#,
        )
        .unwrap();
        let copy = value.clone();
        assert_eq!(copy, value);
        assert_eq!(copy.to_vec(), value.to_vec());
    }

    #[test]
    fn round_trips_nesting_far_deeper_than_any_call_stack_allows() {
        for (open, leaf, close) in [("[", "", "]"), (r#"{"a":"#, "{}", "}")] {
            let source = open.repeat(DEEP) + leaf + &close.repeat(DEEP);
            let value = Value::parse(source.as_bytes()).expect("deep nesting parses");
            assert_eq!(value.to_vec(), source.as_bytes());
            assert!(value.clone() == value);
            // Formatting and dropping walk the same depth.
            assert_eq!(format!("{value:?}").len(), source.len());
        }
    }

    #[test]
    fn reports_unbalanced_deep_nesting_without_overflowing() {
        assert!(Value::parse("[".repeat(DEEP).as_bytes()).is_err());
        assert!(Value::parse(r#"{"a":"#.repeat(DEEP).as_bytes()).is_err());
    }

    #[test]
    fn deep_values_survive_a_small_thread_stack() {
        // The operations run where a per-level call frame has no room at all,
        // which no assertion about the main thread's stack could establish.
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let source = "[".repeat(DEEP) + &"]".repeat(DEEP);
                let value = Value::parse(source.as_bytes()).expect("deep nesting parses");
                let copy = value.clone();
                assert!(copy == value);
                assert_eq!(copy.to_vec().len(), source.len());
            })
            .expect("spawning the test thread")
            .join()
            .expect("the deep value is handled without overflowing");
    }
}
