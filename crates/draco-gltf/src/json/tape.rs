//! Flat, read-only form of a JSON document.
//!
//! A [`Tape`] holds every value of a document as one fixed-size node in a
//! single vector, in document order, with string and number lexemes left in
//! one text buffer that starts with the source itself. Parsing allocates a
//! handful of growing buffers instead of one heap block per key, string,
//! number and container, and dropping frees those few buffers instead of
//! walking a tree. The tree form, [`Value`], is built from a tape only when a
//! caller asks for it or edits the document.

use std::fmt;
use std::mem;
use std::slice;

use super::{write_string, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Kind {
    Null,
    False,
    True,
    Number,
    /// A string copied from the source with no escape in it, so its lexeme is
    /// both its value and its serialization.
    Raw,
    /// A string held decoded, which has to be escaped again to be written.
    Text,
    Array,
    Object,
}

/// One value. Scalars: `a..b` is the lexeme's span in the text buffer.
/// Arrays: `a` is where the array's entry in the slot table starts -- its
/// length, then the node index of each item -- so an item is reached in one
/// step. Objects: `a` is the member count, and each member is a key node
/// followed by its value. For both containers `b` is the index one past the
/// subtree.
///
/// A key's `next` is the index of the member after it, so a lookup walks an
/// object's keys alone and never reads the values it passes over. It is zero
/// on every other node.
#[derive(Clone, Copy, Debug)]
struct Node {
    kind: Kind,
    a: u32,
    b: u32,
    next: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Tape {
    text: String,
    nodes: Vec<Node>,
    slots: Vec<u32>,
    /// How much of `text` is the source document, unchanged.
    source: usize,
}

/// The tape an empty array borrows, so that a missing array and an empty one
/// read the same.
static EMPTY: Tape = Tape {
    text: String::new(),
    nodes: Vec::new(),
    slots: Vec::new(),
    source: 0,
};

fn offset(value: usize) -> u32 {
    u32::try_from(value).expect("a JSON tape addresses at most 4 GiB")
}

impl Tape {
    /// Parses one complete JSON value.
    ///
    /// Like [`Value::parse`] this keeps open containers on the heap rather than
    /// in call frames, so the only bound on nesting is the memory the input
    /// pays for.
    pub(crate) fn parse(input: &[u8]) -> Result<Self, String> {
        if u32::try_from(input.len()).is_err() {
            return Err("JSON document exceeds 4 GiB".into());
        }
        // Everything outside a string is ASCII in valid JSON, so a document
        // that parses is valid UTF-8 as a whole; checking that once up front
        // leaves the string scanner nothing to look at but quotes, backslashes
        // and control characters.
        let source = std::str::from_utf8(input).map_err(|_| "invalid utf8")?;
        let mut parser = Parser {
            input,
            source,
            pos: 0,
            tape: Tape {
                text: String::from(source),
                // About one value per six bytes in glTF JSON.
                nodes: Vec::with_capacity(input.len() / 6 + 1),
                slots: Vec::new(),
                source: input.len(),
            },
        };
        parser.document()?;
        parser.space();
        if parser.pos != input.len() {
            return Err("trailing JSON data".into());
        }
        Ok(parser.tape)
    }

    /// Lays a tree out as a tape. Its strings are kept decoded, so they are
    /// escaped again when written.
    pub(crate) fn from_value(root: &Value) -> Self {
        enum Frame<'a> {
            Array(slice::Iter<'a, Value>, usize, usize),
            Object(slice::Iter<'a, (String, Value)>, usize, Option<usize>),
        }
        let mut tape = Self::default();
        let mut stack: Vec<Frame<'_>> = Vec::new();
        let mut pending = Some(root);
        loop {
            if let Some(value) = pending.take() {
                let index = tape.nodes.len();
                match value {
                    Value::Null => tape.push(Kind::Null, 0, 0),
                    Value::Bool(false) => tape.push(Kind::False, 0, 0),
                    Value::Bool(true) => tape.push(Kind::True, 0, 0),
                    Value::Number(lexeme) => tape.push_text(Kind::Number, lexeme),
                    Value::String(text) => tape.push_text(Kind::Text, text),
                    Value::Array(items) => {
                        let slot = tape.slots.len();
                        tape.slots.push(offset(items.len()));
                        tape.slots.resize(slot + 1 + items.len(), 0);
                        tape.push(Kind::Array, slot, 0);
                        stack.push(Frame::Array(items.iter(), index, slot + 1));
                    }
                    Value::Object(members) => {
                        tape.push(Kind::Object, members.len(), 0);
                        stack.push(Frame::Object(members.iter(), index, None));
                    }
                }
            }
            let item = tape.nodes.len();
            let closed = match stack.last_mut() {
                None => return tape,
                Some(Frame::Array(rest, index, slot)) => match rest.next() {
                    Some(value) => {
                        tape.slots[*slot] = offset(item);
                        *slot += 1;
                        pending = Some(value);
                        continue;
                    }
                    None => *index,
                },
                Some(Frame::Object(rest, index, key)) => {
                    // The previous member's value is complete.
                    if let Some(key) = key.take() {
                        tape.nodes[key].next = offset(item);
                    }
                    match rest.next() {
                        Some((name, value)) => {
                            *key = Some(item);
                            tape.push_text(Kind::Text, name);
                            pending = Some(value);
                            continue;
                        }
                        None => *index,
                    }
                }
            };
            tape.nodes[closed].b = offset(item);
            stack.pop();
        }
    }

    /// The source bytes the tape was parsed from, or nothing for one laid
    /// out from a tree.
    pub(crate) fn source(&self) -> &[u8] {
        &self.text.as_bytes()[..self.source]
    }

    pub(crate) fn root(&self) -> JsonRef<'_> {
        JsonRef {
            tape: self,
            index: 0,
        }
    }

    fn push(&mut self, kind: Kind, a: usize, b: usize) {
        self.nodes.push(Node {
            kind,
            a: offset(a),
            b: offset(b),
            next: 0,
        });
    }

    fn push_text(&mut self, kind: Kind, text: &str) {
        let start = self.text.len();
        self.text.push_str(text);
        self.push(kind, start, self.text.len());
    }

    fn span(&self, node: Node) -> &str {
        &self.text[node.a as usize..node.b as usize]
    }
}

/// A borrowed JSON value inside a parsed document.
///
/// It is a position in the document's flat node table, so it is `Copy`, and
/// every accessor reads the table directly: nothing is decoded or allocated to
/// look at a value.
#[derive(Clone, Copy)]
pub struct JsonRef<'a> {
    tape: &'a Tape,
    index: u32,
}

/// What a position past the end of a tape reads as, which is how the null a
/// lookup falls back to is represented.
const NULL: Node = Node {
    kind: Kind::Null,
    a: 0,
    b: 0,
    next: 0,
};

impl Default for JsonRef<'_> {
    /// A JSON null that belongs to no document.
    fn default() -> Self {
        Self {
            tape: &EMPTY,
            index: 0,
        }
    }
}

impl<'a> JsonRef<'a> {
    fn node(self) -> Node {
        self.tape
            .nodes
            .get(self.index as usize)
            .copied()
            .unwrap_or(NULL)
    }
    /// Index one past this value's subtree.
    fn end(self) -> u32 {
        let node = self.node();
        match node.kind {
            Kind::Array | Kind::Object => node.b,
            _ => self.index + 1,
        }
    }
    /// Returns whether this value is JSON null.
    pub fn is_null(self) -> bool {
        self.node().kind == Kind::Null
    }
    /// Returns whether this value is an object.
    pub fn is_object(self) -> bool {
        self.node().kind == Kind::Object
    }
    /// Returns whether this value is an array.
    pub fn is_array(self) -> bool {
        self.node().kind == Kind::Array
    }
    /// Returns the boolean when this value is one.
    pub fn as_bool(self) -> Option<bool> {
        match self.node().kind {
            Kind::True => Some(true),
            Kind::False => Some(false),
            _ => None,
        }
    }
    /// Borrows the string when this value is a string.
    pub fn as_str(self) -> Option<&'a str> {
        let node = self.node();
        matches!(node.kind, Kind::Raw | Kind::Text).then(|| self.tape.span(node))
    }
    /// Borrows a number's lexeme exactly as the document spells it.
    pub fn as_number(self) -> Option<&'a str> {
        let node = self.node();
        (node.kind == Kind::Number).then(|| self.tape.span(node))
    }
    /// Parses a non-negative integer without changing its stored lexeme.
    ///
    /// Indices are most of what a glTF reader parses, so the digits are read
    /// directly. A lexeme here already follows the JSON grammar, which leaves
    /// the cases `str::parse` would refuse as a sign, a fraction, an exponent
    /// or overflow, and each of those is refused here too.
    pub fn as_u64(self) -> Option<u64> {
        let node = self.node();
        if node.kind != Kind::Number {
            return None;
        }
        let digits = self
            .tape
            .text
            .as_bytes()
            .get(node.a as usize..node.b as usize)?;
        digits.iter().try_fold(0u64, |value, &digit| {
            if !digit.is_ascii_digit() {
                return None;
            }
            value.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
        })
    }
    /// Parses a JSON number as `f64` without changing its stored lexeme.
    pub fn as_f64(self) -> Option<f64> {
        self.as_number()?.parse().ok()
    }
    /// Returns the members when this value is an object.
    pub fn as_object(self) -> Option<JsonObject<'a>> {
        let node = self.node();
        (node.kind == Kind::Object).then_some(JsonObject {
            tape: self.tape,
            first: self.index + 1,
            len: node.a,
        })
    }
    /// Returns the items when this value is an array.
    pub fn as_array(self) -> Option<JsonArray<'a>> {
        let node = self.node();
        (node.kind == Kind::Array).then(|| {
            let start = node.a as usize + 1;
            let len = self.tape.slots[start - 1] as usize;
            JsonArray {
                tape: self.tape,
                items: &self.tape.slots[start..start + len],
            }
        })
    }
    /// Looks up an object member by key.
    pub fn get(self, key: &str) -> Option<JsonRef<'a>> {
        self.as_object()?.get(key)
    }
    /// Copies this value out as an owned tree.
    pub fn to_value(self) -> Value {
        // A container being rebuilt: what is left to copy and what has been
        // copied so far. Object frames also carry the key whose value is
        // being copied.
        enum Frame<'a> {
            Array(JsonItems<'a>, Vec<Value>),
            Object(JsonMembers<'a>, Vec<(String, Value)>, &'a str),
        }
        let mut stack: Vec<Frame<'_>> = Vec::new();
        let mut source = Some(self);
        let mut done: Option<Value> = None;
        loop {
            if let Some(value) = source.take() {
                let node = value.node();
                match node.kind {
                    Kind::Null => done = Some(Value::Null),
                    Kind::False => done = Some(Value::Bool(false)),
                    Kind::True => done = Some(Value::Bool(true)),
                    Kind::Number => done = Some(Value::Number(value.tape.span(node).into())),
                    Kind::Raw | Kind::Text => {
                        done = Some(Value::String(value.tape.span(node).into()));
                    }
                    Kind::Array => {
                        let items = value.as_array().unwrap_or_default();
                        stack.push(Frame::Array(items.iter(), Vec::with_capacity(items.len())));
                    }
                    Kind::Object => {
                        let members = value.as_object().unwrap_or_default();
                        stack.push(Frame::Object(
                            members.iter(),
                            Vec::with_capacity(members.len()),
                            "",
                        ));
                    }
                }
            }
            let finished = match stack.last_mut() {
                None => return done.expect("the root value is copied before the stack empties"),
                Some(Frame::Array(rest, out)) => {
                    if let Some(value) = done.take() {
                        out.push(value);
                    }
                    source = rest.next();
                    source.is_none()
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
                        Frame::Array(_, out) => Value::Array(out),
                        Frame::Object(_, out, _) => Value::Object(out),
                    },
                );
            }
        }
    }
    /// Serializes this value as whitespace-free JSON.
    pub fn to_vec(self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }
    /// Writes the subtree in one pass over its nodes, which are already in
    /// document order; the only state is the stack of open containers.
    pub(crate) fn write(self, out: &mut Vec<u8>) {
        // An open container: the index its subtree ends at, whether it is an
        // object, whether its next node is a key, and whether a separator is
        // owed before its next entry.
        struct Open {
            end: u32,
            object: bool,
            key_next: bool,
            separate: bool,
        }
        let nodes = &self.tape.nodes;
        let mut open: Vec<Open> = Vec::new();
        for index in self.index..self.end() {
            while let Some(container) = open.last() {
                if container.end != index {
                    break;
                }
                out.push(if container.object { b'}' } else { b']' });
                open.pop();
            }
            let mut key = false;
            if let Some(container) = open.last_mut() {
                if (!container.object || container.key_next)
                    && mem::replace(&mut container.separate, true)
                {
                    out.push(b',');
                }
                if container.object {
                    key = container.key_next;
                    container.key_next = !key;
                }
            }
            let node = nodes[index as usize];
            match node.kind {
                Kind::Null => out.extend_from_slice(b"null"),
                Kind::False => out.extend_from_slice(b"false"),
                Kind::True => out.extend_from_slice(b"true"),
                Kind::Number => out.extend_from_slice(self.tape.span(node).as_bytes()),
                Kind::Raw => {
                    out.push(b'"');
                    out.extend_from_slice(self.tape.span(node).as_bytes());
                    out.push(b'"');
                }
                Kind::Text => write_string(out, self.tape.span(node)),
                Kind::Array | Kind::Object => {
                    let object = node.kind == Kind::Object;
                    out.push(if object { b'{' } else { b'[' });
                    open.push(Open {
                        end: node.b,
                        object,
                        key_next: true,
                        separate: false,
                    });
                }
            }
            if key {
                out.push(b':');
            }
        }
        while let Some(container) = open.pop() {
            out.push(if container.object { b'}' } else { b']' });
        }
    }
}

impl PartialEq for JsonRef<'_> {
    /// Compares the two subtrees node by node. Document order and the entry
    /// count of every container fix a tree's shape, so equal node sequences
    /// are equal trees, and the comparison needs no stack.
    fn eq(&self, other: &Self) -> bool {
        let (left, right) = (self.index..self.end(), other.index..other.end());
        if left.len() != right.len() {
            return false;
        }
        left.zip(right).all(|(a, b)| {
            let (a, b) = (
                JsonRef {
                    tape: self.tape,
                    index: a,
                },
                JsonRef {
                    tape: other.tape,
                    index: b,
                },
            );
            let (x, y) = (a.node(), b.node());
            match (x.kind, y.kind) {
                (Kind::Raw | Kind::Text, Kind::Raw | Kind::Text) | (Kind::Number, Kind::Number) => {
                    a.tape.span(x) == b.tape.span(y)
                }
                (Kind::Array, Kind::Array) => {
                    a.as_array().map(|v| v.len()) == b.as_array().map(|v| v.len())
                }
                (Kind::Object, Kind::Object) => x.a == y.a,
                (x, y) => x == y,
            }
        })
    }
}

impl fmt::Debug for JsonRef<'_> {
    /// Renders the value as JSON text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.to_vec()))
    }
}

/// The members of a JSON object, in document order.
#[derive(Clone, Copy)]
pub struct JsonObject<'a> {
    tape: &'a Tape,
    first: u32,
    len: u32,
}

impl Default for JsonObject<'_> {
    fn default() -> Self {
        Self {
            tape: &EMPTY,
            first: 0,
            len: 0,
        }
    }
}

impl<'a> JsonObject<'a> {
    /// Returns the number of members, duplicates included.
    pub fn len(self) -> usize {
        self.len as usize
    }
    /// Returns whether the object has no members.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    /// Looks up the first member with this key.
    ///
    /// Every field a reader asks for is found here, so the scan compares key
    /// bytes in place rather than going through [`JsonRef::as_str`], whose
    /// `str` slicing checks character boundaries on every key it passes.
    pub fn get(self, key: &str) -> Option<JsonRef<'a>> {
        let nodes = &self.tape.nodes;
        let text = self.tape.text.as_bytes();
        let key = key.as_bytes();
        let mut index = self.first as usize;
        for _ in 0..self.len {
            let Some(name) = nodes.get(index) else {
                break;
            };
            // Most keys differ from the one asked for in length, which is
            // decided before any text is looked at.
            let (start, end) = (name.a as usize, name.b as usize);
            if end.wrapping_sub(start) == key.len() && text.get(start..end) == Some(key) {
                return Some(JsonRef {
                    tape: self.tape,
                    index: index as u32 + 1,
                });
            }
            index = name.next as usize;
        }
        None
    }
    /// Iterates over the members as key and value pairs.
    pub fn iter(self) -> JsonMembers<'a> {
        JsonMembers {
            tape: self.tape,
            index: self.first,
            left: self.len,
        }
    }
}

impl<'a> IntoIterator for JsonObject<'a> {
    type Item = (&'a str, JsonRef<'a>);
    type IntoIter = JsonMembers<'a>;
    fn into_iter(self) -> JsonMembers<'a> {
        self.iter()
    }
}

/// Iterator over the members of a [`JsonObject`].
#[derive(Clone)]
pub struct JsonMembers<'a> {
    tape: &'a Tape,
    index: u32,
    left: u32,
}

impl<'a> Iterator for JsonMembers<'a> {
    type Item = (&'a str, JsonRef<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let key = JsonRef {
            tape: self.tape,
            index: self.index,
        };
        let value = JsonRef {
            tape: self.tape,
            index: self.index + 1,
        };
        self.index = key.node().next;
        Some((key.as_str().unwrap_or_default(), value))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left as usize, Some(self.left as usize))
    }
}

impl ExactSizeIterator for JsonMembers<'_> {}

/// The items of a JSON array, each reachable by position in one step.
#[derive(Clone, Copy)]
pub struct JsonArray<'a> {
    tape: &'a Tape,
    items: &'a [u32],
}

impl Default for JsonArray<'_> {
    fn default() -> Self {
        Self {
            tape: &EMPTY,
            items: &[],
        }
    }
}

impl<'a> JsonArray<'a> {
    /// Returns the number of items.
    pub fn len(self) -> usize {
        self.items.len()
    }
    /// Returns whether the array has no items.
    pub fn is_empty(self) -> bool {
        self.items.is_empty()
    }
    /// Returns the item at `index`.
    pub fn get(self, index: usize) -> Option<JsonRef<'a>> {
        self.items.get(index).map(|&index| JsonRef {
            tape: self.tape,
            index,
        })
    }
    /// Iterates over the items in order.
    pub fn iter(self) -> JsonItems<'a> {
        JsonItems {
            tape: self.tape,
            items: self.items.iter(),
        }
    }
}

impl<'a> IntoIterator for JsonArray<'a> {
    type Item = JsonRef<'a>;
    type IntoIter = JsonItems<'a>;
    fn into_iter(self) -> JsonItems<'a> {
        self.iter()
    }
}

/// Iterator over the items of a [`JsonArray`].
#[derive(Clone)]
pub struct JsonItems<'a> {
    tape: &'a Tape,
    items: slice::Iter<'a, u32>,
}

impl<'a> Iterator for JsonItems<'a> {
    type Item = JsonRef<'a>;
    fn next(&mut self) -> Option<JsonRef<'a>> {
        self.items.next().map(|&index| JsonRef {
            tape: self.tape,
            index,
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

impl DoubleEndedIterator for JsonItems<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.items.next_back().map(|&index| JsonRef {
            tape: self.tape,
            index,
        })
    }
}

impl ExactSizeIterator for JsonItems<'_> {}

/// A container the parser has opened but not yet closed.
struct Frame {
    node: usize,
    /// Arrays: where this array's item indexes start on the shared pending
    /// stack. Objects: the members closed so far.
    entries: usize,
    object: bool,
    /// Objects: the node of the key whose value is being parsed.
    key: usize,
}

struct Parser<'a> {
    input: &'a [u8],
    /// The input as text, which the string decoder copies runs out of.
    source: &'a str,
    pos: usize,
    tape: Tape,
}

impl Parser<'_> {
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
    /// Parses one value, holding open containers on the heap.
    ///
    /// The items of every open array wait on one shared stack as node
    /// indexes, and an array that closes moves its own off the top into the
    /// slot table, so each array's items end up contiguous there however the
    /// arrays nest.
    fn document(&mut self) -> Result<(), String> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut pending: Vec<u32> = Vec::new();
        'value: loop {
            self.space();
            if stack.last().is_some_and(|frame| !frame.object) {
                pending.push(offset(self.tape.nodes.len()));
            }
            match self.input.get(self.pos).copied() {
                Some(open @ (b'{' | b'[')) => {
                    self.pos += 1;
                    let node = self.tape.nodes.len();
                    let object = open == b'{';
                    if object {
                        self.tape.push(Kind::Object, 0, 0);
                    } else {
                        let slot = self.tape.slots.len();
                        self.tape.push(Kind::Array, slot, 0);
                    }
                    if self.take(if object { b'}' } else { b']' }) {
                        if !object {
                            self.tape.slots.push(0);
                        }
                        self.tape.nodes[node].b = offset(node + 1);
                    } else {
                        stack.push(Frame {
                            node,
                            entries: if object { 0 } else { pending.len() },
                            object,
                            key: node + 1,
                        });
                        if object {
                            self.key()?;
                        }
                        continue 'value;
                    }
                }
                Some(b'"') => self.string()?,
                Some(b't') => self.literal(b"true", Kind::True)?,
                Some(b'f') => self.literal(b"false", Kind::False)?,
                Some(b'n') => self.literal(b"null", Kind::Null)?,
                Some(b'-' | b'0'..=b'9') => self.number()?,
                _ => return Err("expected JSON value".into()),
            }
            // Close every container that ends here.
            loop {
                let Some(frame) = stack.last_mut() else {
                    return Ok(());
                };
                if frame.object {
                    frame.entries += 1;
                    self.tape.nodes[frame.key].next = offset(self.tape.nodes.len());
                }
                let object = frame.object;
                if self.take(if object { b'}' } else { b']' }) {
                    let frame = stack.pop().expect("a frame was just observed on the stack");
                    let end = offset(self.tape.nodes.len());
                    if object {
                        self.tape.nodes[frame.node].a = offset(frame.entries);
                    } else {
                        let slot = self.tape.slots.len();
                        self.tape.slots.push(offset(pending.len() - frame.entries));
                        self.tape.slots.extend(pending.drain(frame.entries..));
                        self.tape.nodes[frame.node].a = offset(slot);
                    }
                    self.tape.nodes[frame.node].b = end;
                    continue;
                }
                if !self.take(b',') {
                    return Err(if object {
                        "missing object comma"
                    } else {
                        "missing array comma"
                    }
                    .into());
                }
                if object {
                    if let Some(frame) = stack.last_mut() {
                        frame.key = self.tape.nodes.len();
                    }
                    self.key()?;
                }
                continue 'value;
            }
        }
    }
    /// Consumes one object key and the colon that must follow it.
    fn key(&mut self) -> Result<(), String> {
        self.space();
        if self.input.get(self.pos) != Some(&b'"') {
            return Err("object key is not a string".into());
        }
        self.string()?;
        if !self.take(b':') {
            return Err("missing object colon".into());
        }
        Ok(())
    }
    fn literal(&mut self, s: &[u8], kind: Kind) -> Result<(), String> {
        if self.input.get(self.pos..self.pos + s.len()) == Some(s) {
            self.pos += s.len();
            self.tape.push(kind, 0, 0);
            Ok(())
        } else {
            Err("invalid JSON literal".into())
        }
    }
    /// Scans a string. The input is valid UTF-8 as a whole, so only quotes,
    /// backslashes and control characters need a look; a string without a
    /// backslash is its own lexeme and is not copied.
    fn string(&mut self) -> Result<(), String> {
        self.pos += 1;
        let start = self.pos;
        loop {
            match *self.input.get(self.pos).ok_or("unterminated string")? {
                b'"' => {
                    self.tape.push(Kind::Raw, start, self.pos);
                    self.pos += 1;
                    return Ok(());
                }
                b'\\' => return self.escaped(start),
                0..=0x1f => return Err("control character in string".into()),
                _ => self.pos += 1,
            }
        }
    }
    /// Decodes a string that has an escape in it onto the end of the text
    /// buffer.
    fn escaped(&mut self, start: usize) -> Result<(), String> {
        let from = self.tape.text.len();
        // Every run copied ends just before an ASCII quote or backslash, so
        // it is a whole number of UTF-8 sequences of the source.
        let mut run = start;
        loop {
            let b = *self.input.get(self.pos).ok_or("unterminated string")?;
            match b {
                b'"' | b'\\' => {
                    self.tape.text.push_str(&self.source[run..self.pos]);
                    self.pos += 1;
                    if b == b'"' {
                        let to = self.tape.text.len();
                        self.tape.push(Kind::Text, from, to);
                        return Ok(());
                    }
                    let escape = *self.input.get(self.pos).ok_or("bad escape")?;
                    self.pos += 1;
                    let ch = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode()?,
                        _ => return Err("invalid escape".into()),
                    };
                    self.tape.text.push(ch);
                    run = self.pos;
                }
                0..=0x1f => return Err("control character in string".into()),
                _ => self.pos += 1,
            }
        }
    }
    /// Decodes a `\u` escape, and the low surrogate escape that must follow a
    /// high one.
    fn unicode(&mut self) -> Result<char, String> {
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
                0x1_0000 + (u32::from(first - 0xd800) << 10) + u32::from(second - 0xdc00)
            }
            0xdc00..=0xdfff => return Err("unpaired low surrogate".into()),
            value => u32::from(value),
        };
        char::from_u32(scalar).ok_or_else(|| "invalid unicode scalar".into())
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
    fn digits(&mut self) {
        while self.input.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
    }
    fn number(&mut self) -> Result<(), String> {
        let start = self.pos;
        if self.input.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.input.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                self.digits();
            }
            _ => return Err("invalid number".into()),
        }
        if self.input.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !self.input.get(self.pos).is_some_and(u8::is_ascii_digit) {
                return Err("invalid number fraction".into());
            }
            self.digits();
        }
        if matches!(self.input.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.input.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.input.get(self.pos).is_some_and(u8::is_ascii_digit) {
                return Err("invalid number exponent".into());
            }
            self.digits();
        }
        self.tape.push(Kind::Number, start, self.pos);
        Ok(())
    }
}
