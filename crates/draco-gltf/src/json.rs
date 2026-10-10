//! Small, dependency-free JSON DOM used by the lossless glTF model.

use std::fmt;
use std::mem;
use std::ops::{Index, IndexMut};
use std::slice;

mod tape;

pub use tape::{JsonArray, JsonIndex, JsonItems, JsonMembers, JsonObject, JsonRef};
pub(crate) use tape::{Patches, Tape};

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
        Tape::parse(input).map(|tape| tape.root().to_value())
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

/// Writes `value` as a JSON string, copying the runs between the bytes that
/// need an escape. Those are all ASCII, and no byte of a multi-byte UTF-8
/// sequence is, so the runs are whole characters.
fn write_string(out: &mut Vec<u8>, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    let bytes = value.as_bytes();
    let mut run = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        let short: &[u8] = match byte {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0..=0x1f => b"",
            _ => continue,
        };
        out.extend_from_slice(&bytes[run..index]);
        if short.is_empty() {
            out.extend_from_slice(b"\\u00");
            out.push(HEX[usize::from(byte >> 4)]);
            out.push(HEX[usize::from(byte & 0xf)]);
        } else {
            out.extend_from_slice(short);
        }
        run = index + 1;
    }
    out.extend_from_slice(&bytes[run..]);
    out.push(b'"');
}
#[cfg(test)]
mod tests {
    use super::{mem, JsonRef, Tape, Value};

    /// Whether a tape value and a tree hold the same document, reached through
    /// the tape's own accessors: items by position, members in order.
    fn same(tape: JsonRef<'_>, tree: &Value) -> bool {
        let mut work = vec![(tape, tree)];
        while let Some((tape, tree)) = work.pop() {
            let equal = match tree {
                Value::Null => tape.is_null(),
                Value::Bool(v) => tape.as_bool() == Some(*v),
                Value::Number(v) => tape.as_number() == Some(v.as_str()),
                Value::String(v) => tape.as_str() == Some(v.as_str()),
                Value::Array(items) => tape.as_array().is_some_and(|array| {
                    array.len() == items.len()
                        && items.iter().enumerate().all(|(index, item)| {
                            let entry = array.get(index).expect("an item at every position");
                            work.push((entry, item));
                            true
                        })
                }),
                Value::Object(members) => tape.as_object().is_some_and(|object| {
                    object.len() == members.len()
                        && object
                            .iter()
                            .zip(members)
                            .all(|((key, entry), (name, member))| {
                                work.push((entry, member));
                                key == name
                            })
                }),
            };
            if !equal {
                return false;
            }
        }
        true
    }

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

            let scalars = [
                "1",
                "-2.5e3",
                "true",
                "false",
                "null",
                r#""s""#,
                r#""a\"b\u00e9\/\n""#,
                "\"\u{e9}\u{1f680}\"",
                "[]",
                "{}",
            ];
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

                // The tape the tree was built from, and one laid out from the
                // tree, both read and write as the tree does.
                let parsed = Tape::parse(source.as_bytes()).expect("the tree parsed");
                let laid_out = Tape::from_value(&value);
                for tape in [&parsed, &laid_out] {
                    assert!(
                        same(tape.root(), &value),
                        "seed {seed} tape reads differently"
                    );
                    assert_eq!(
                        tape.root().to_vec(),
                        text,
                        "seed {seed} tape writes differently"
                    );
                    assert_eq!(
                        tape.root().to_value(),
                        value,
                        "seed {seed} tape copies out wrong"
                    );
                }
                assert_eq!(
                    parsed.root(),
                    laid_out.root(),
                    "seed {seed} tapes compare unequal"
                );
            }
        }
        // A generator that stopped producing parsable documents would make the
        // round trip vacuous.
        assert!(accepted > 3_000, "only {accepted} documents parsed");
    }

    #[test]
    fn tape_equality_distinguishes_what_the_tree_does() {
        let parse = |text: &str| Tape::parse(text.as_bytes()).unwrap();
        let pairs = [
            (r#"{"a":[1,{"b":null}]}"#, r#"{"a":[1,{"b":null}]}"#, true),
            // An escaped spelling is the same string.
            (r#"["\u0061\/"]"#, r#"["a/"]"#, true),
            ("1", r#""1""#, false),
            ("1", "1.0", false),
            (r#"{"a":1,"b":2}"#, r#"{"b":2,"a":1}"#, false),
            ("[1,2]", "[1,2,3]", false),
            ("[[1],2]", "[[1,2]]", false),
            ("[]", "{}", false),
            ("true", "false", false),
        ];
        for (a, b, equal) in pairs {
            assert_eq!(parse(a).root() == parse(b).root(), equal, "{a} against {b}");
        }
    }

    #[test]
    fn strings_write_every_control_character_escaped() {
        let text: String = (0u8..0x20)
            .map(char::from)
            .chain("a\"\\é/".chars())
            .collect();
        let mut expected = String::from("\"");
        for byte in 0u8..0x20 {
            expected.push_str(match byte {
                b'\n' => "\\n",
                b'\r' => "\\r",
                b'\t' => "\\t",
                _ => "",
            });
            if !matches!(byte, b'\n' | b'\r' | b'\t') {
                expected.push_str(&format!("\\u{byte:04x}"));
            }
        }
        expected.push_str("a\\\"\\\\é/\"");
        let value = Value::String(text.clone());
        assert_eq!(String::from_utf8(value.to_vec()).unwrap(), expected);
        // The tape writes the same, and reads the text back.
        let tape = Tape::from_value(&value);
        assert_eq!(tape.root().to_vec(), value.to_vec());
        assert_eq!(Value::parse(expected.as_bytes()).unwrap(), value);
    }

    #[test]
    fn tape_integers_read_as_str_parse_reads_them() {
        let lexemes = [
            "0",
            "7",
            "12",
            "18446744073709551615",
            "18446744073709551616",
            "99999999999999999999",
            "-0",
            "-1",
            "1.0",
            "1e2",
            "1E+2",
        ];
        let source = format!("[{}]", lexemes.join(","));
        let tape = Tape::parse(source.as_bytes()).unwrap();
        let items = tape.root().as_array().unwrap();
        for (item, lexeme) in items.iter().zip(lexemes) {
            assert_eq!(item.as_u64(), lexeme.parse().ok(), "{lexeme}");
        }
        assert_eq!(Tape::parse(br#""5""#).unwrap().root().as_u64(), None);
    }

    #[test]
    fn tape_reads_items_by_position_across_nested_arrays() {
        let tape = Tape::parse(br#"[[1,[2,3]],[],[4],{"k":[5,6]},7]"#).unwrap();
        let root = tape.root().as_array().unwrap();
        assert_eq!(root.len(), 5);
        let first = root.get(0).unwrap().as_array().unwrap();
        assert_eq!(
            first
                .get(1)
                .unwrap()
                .as_array()
                .unwrap()
                .get(1)
                .unwrap()
                .as_u64(),
            Some(3)
        );
        assert!(root.get(1).unwrap().as_array().unwrap().is_empty());
        assert_eq!(
            root.get(2)
                .unwrap()
                .as_array()
                .unwrap()
                .get(0)
                .unwrap()
                .as_u64(),
            Some(4)
        );
        let nested = root.get(3).unwrap().get("k").unwrap().as_array().unwrap();
        assert_eq!(
            nested
                .iter()
                .filter_map(JsonRef::as_u64)
                .collect::<Vec<_>>(),
            [5, 6]
        );
        assert_eq!(root.get(4).unwrap().as_u64(), Some(7));
        assert!(root.get(5).is_none());
        // A missing value reads as null.
        assert!(JsonRef::default().is_null());
        assert!(JsonRef::default().as_array().is_none());
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
                let tape = Tape::parse(source.as_bytes()).expect("deep nesting parses");
                let laid_out = Tape::from_value(&value);
                assert!(tape.root() == laid_out.root());
                assert_eq!(laid_out.root().to_vec().len(), source.len());
                assert!(tape.root().to_value() == value);
            })
            .expect("spawning the test thread")
            .join()
            .expect("the deep value is handled without overflowing");
    }
    /// `at` reads a path the way indexing the tree does: every step that is
    /// there, missing, past the end of an array, or taken into a value of the
    /// wrong kind lands on the same value.
    #[test]
    fn at_follows_a_path_the_way_tree_indexing_does() {
        enum Step {
            Key(&'static str),
            Item(usize),
        }
        use Step::{Item, Key};
        let source = r#"{"meshes":[{"name":"a","primitives":[{"attributes":{"POSITION":0}}]},{"name":"b"}],"asset":{"version":"2.0"},"n":3}"#;
        let tape = Tape::parse(source.as_bytes()).unwrap();
        let tree = tape.root().to_value();
        let paths: [&[Step]; 9] = [
            &[Key("meshes"), Item(0), Key("name")],
            &[
                Key("meshes"),
                Item(0),
                Key("primitives"),
                Item(0),
                Key("attributes"),
                Key("POSITION"),
            ],
            &[Key("meshes"), Item(1)],
            &[Key("meshes"), Item(2), Key("name")],
            &[Key("missing"), Key("deeper"), Item(0)],
            &[Key("n"), Key("x")],
            &[Key("asset"), Item(0)],
            &[Key("meshes"), Key("name")],
            &[],
        ];
        for (number, path) in paths.iter().enumerate() {
            let (mut at, mut indexed) = (tape.root(), &tree);
            for step in path.iter() {
                match *step {
                    Key(key) => (at, indexed) = (at.at(key), &indexed[key]),
                    Item(item) => (at, indexed) = (at.at(item), &indexed[item]),
                }
            }
            assert!(same(at, indexed), "path {number}");
        }
        assert!(tape.root().at("missing").at(0).is_null());
        let key = String::from("asset");
        assert_eq!(tape.root().at(&key).at("version").as_str(), Some("2.0"));
    }
    /// RFC 6901's own examples (section 5): every pointer resolves to the
    /// value the RFC gives it, escapes and the empty key included. Malformed
    /// pointers and positions an array does not have resolve to nothing.
    #[test]
    fn pointer_resolves_rfc_6901s_examples() {
        let source = r#"{"foo":["bar","baz"],"":0,"a/b":1,"c%d":2,"e^f":3,"g|h":4,"i\\j":5,"k\"l":6," ":7,"m~n":8}"#;
        let tape = Tape::parse(source.as_bytes()).unwrap();
        let root = tape.root();
        assert!(root.pointer("").is_some_and(|whole| whole.is_object()));
        assert_eq!(
            root.pointer("/foo")
                .and_then(|foo| foo.as_array())
                .map(|a| a.len()),
            Some(2)
        );
        assert_eq!(root.pointer("/foo/0").and_then(|v| v.as_str()), Some("bar"));
        for (pointer, expected) in [
            ("/", 0),
            ("/a~1b", 1),
            ("/c%d", 2),
            ("/e^f", 3),
            ("/g|h", 4),
            ("/i\\j", 5),
            ("/k\"l", 6),
            ("/ ", 7),
            ("/m~0n", 8),
        ] {
            assert_eq!(
                root.pointer(pointer).and_then(|v| v.as_u64()),
                Some(expected),
                "{pointer}"
            );
        }
        for pointer in [
            "foo",
            "/foo/01",
            "/foo/-",
            "/foo/2",
            "/foo/+1",
            "/m~2n",
            "/m~",
            "/missing/0",
        ] {
            assert!(root.pointer(pointer).is_none(), "{pointer}");
        }
        // The shape KHR_animation_pointer's targets take.
        let gltf = Tape::parse(br#"{"nodes":[{"translation":[1,2,3]}]}"#).unwrap();
        let translation = gltf
            .root()
            .pointer("/nodes/0/translation")
            .and_then(|v| v.as_array());
        assert_eq!(translation.map(|a| a.len()), Some(3));
    }
    /// A position past the tape is null when written as well as when read:
    /// `at` lands there on a miss, and `Debug` writes through the same path.
    #[test]
    fn a_missing_at_writes_and_formats_as_null() {
        let tape = Tape::parse(br#"{"a":1}"#).unwrap();
        let missing = tape.root().at("nope").at(3);
        assert_eq!(missing.to_vec(), b"null");
        assert_eq!(format!("{missing:?}"), "null");
        assert_eq!(JsonRef::default().to_vec(), b"null");
    }

    /// A `\u` escape is four hex digits; a sign is not one, though
    /// `u16::from_str_radix` would take a leading `+`.
    #[test]
    fn a_unicode_escape_takes_four_hex_digits_and_nothing_else() {
        assert_eq!(
            Tape::parse(br#""\u0041""#).unwrap().root().as_str(),
            Some("A")
        );
        for bad in [
            &br#""\u+041""#[..],
            br#""\u-041""#,
            br#""\u 041""#,
            br#""\u004""#,
        ] {
            assert!(
                Tape::parse(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
