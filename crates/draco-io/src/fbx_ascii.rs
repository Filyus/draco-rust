//! Reader for the ASCII FBX container.
//!
//! ASCII FBX carries the same node tree as the binary container, so this
//! module's only job is to produce [`FbxNode`](crate::fbx_container::FbxNode)s indistinguishable from the ones
//! [`crate::fbx_container`] decodes. Everything above that -- objects,
//! connections, materials, skins, animation, layer elements -- is then shared,
//! which is what keeps the two containers from drifting apart semantically.
//!
//! Two differences from the binary form are not merely syntactic and are
//! normalized here rather than left for consumers to discover:
//!
//! * Object names are written `"Class::Name"`, where the binary container
//!   writes `"Name\0\x01Class"` -- reversed, with a different separator.
//! * Values carry no type tag. A number is typed by how it is written, so an
//!   array of whole numbers is indistinguishable from an integer array.
//!
//! FBX 6100 and later are accepted, matching the binary reader's floor: the
//! 6100 name-keyed object model is read by the shared scene passes, and
//! earlier versions use layouts neither container here decodes.

use std::io;

use crate::fbx_ascii_syntax::{
    array_element_type, decode_base64, is_base64_node, normalize_object_name, number_property,
    parse_ascii_bool, properties70_type_is_integral, ArrayElement,
};
use crate::fbx_container::{FbxNode, FbxProperty};
use crate::fbx_options::FbxReadOptions;

/// Returns whether the input looks like an ASCII FBX document.
///
/// Autodesk writes a `; FBX` comment header, but not every writer does, so a
/// leading `FBXHeaderExtension:` is accepted too. The check is deliberately
/// narrow: anything that is not clearly ASCII FBX should reach the binary
/// reader and be rejected there with its own message.
pub fn is_ascii_fbx(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(256)];
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start();
    trimmed.starts_with("; FBX")
        || trimmed.starts_with(";FBX")
        || trimmed.starts_with("FBXHeaderExtension:")
}

/// Parses an ASCII FBX document into the same node tree the binary reader
/// produces.
pub fn parse_ascii_nodes(bytes: &[u8], options: &FbxReadOptions) -> io::Result<Vec<FbxNode>> {
    if bytes.len() as u64 > options.limits.max_file_bytes {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!(
                "FBX: ASCII document is {} bytes, over the {} byte limit",
                bytes.len(),
                options.limits.max_file_bytes
            ),
        ));
    }
    let mut parser = Parser {
        input: bytes,
        pos: 0,
        options,
        nodes_read: 0,
        array_bytes: 0,
    };
    let nodes = parser.parse_document()?;
    check_version(&nodes)?;
    Ok(nodes)
}

/// The version an ASCII document declares in its header extension.
pub fn declared_version(nodes: &[FbxNode]) -> Option<u32> {
    nodes
        .iter()
        .find(|node| node.name == "FBXHeaderExtension")
        .and_then(|header| header.children.iter().find(|c| c.name == "FBXVersion"))
        .and_then(|node| match node.properties.first() {
            Some(FbxProperty::I32(value)) => Some(*value),
            Some(FbxProperty::I64(value)) => Some(*value as i32),
            Some(FbxProperty::F64(value)) => Some(*value as i32),
            _ => None,
        })
        .map(|version| version as u32)
}

/// Rejects pre-6100 documents, whose object layouts predate the name-keyed
/// model the scene passes read.
fn check_version(nodes: &[FbxNode]) -> io::Result<()> {
    match declared_version(nodes).map(|version| version as i32) {
        Some(version) if version < 6100 => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "FBX: ASCII version {version} predates the 6100 object model, which is not read"
            ),
        )),
        _ => Ok(()),
    }
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    options: &'a FbxReadOptions,
    nodes_read: u64,
    array_bytes: u64,
}

impl<'a> Parser<'a> {
    fn error(&self, message: impl AsRef<str>) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "FBX: at byte {} of ASCII document: {}",
                self.pos,
                message.as_ref()
            ),
        )
    }

    fn parse_document(&mut self) -> io::Result<Vec<FbxNode>> {
        let mut nodes = Vec::new();
        loop {
            self.skip_trivia();
            if self.pos >= self.input.len() {
                return Ok(nodes);
            }
            if self.input[self.pos] == b'}' {
                return Err(self.error("unmatched closing brace"));
            }
            nodes.push(self.parse_node(0)?);
        }
    }

    /// Skips whitespace and `;` comments, which run to the end of the line.
    fn skip_trivia(&mut self) {
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b';' => {
                    while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn parse_node(&mut self, depth: u32) -> io::Result<FbxNode> {
        if depth > self.options.limits.max_depth {
            return Err(self.error(format!(
                "node nesting deeper than the {} level limit",
                self.options.limits.max_depth
            )));
        }
        self.nodes_read += 1;
        if self.nodes_read > self.options.limits.max_nodes {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "FBX: ASCII document has more than {} nodes",
                    self.options.limits.max_nodes
                ),
            ));
        }

        let name = self.parse_identifier()?;
        self.skip_trivia();
        if self.pos >= self.input.len() || self.input[self.pos] != b':' {
            return Err(self.error(format!("expected ':' after node name {name}")));
        }
        self.pos += 1;

        let mut node = FbxNode {
            name,
            properties: Vec::new(),
            children: Vec::new(),
        };
        let declared_array_len = self.parse_properties(&mut node)?;
        if node.name == "P" {
            coerce_typed_property(&mut node);
        }

        self.skip_trivia();
        if self.pos < self.input.len() && self.input[self.pos] == b'{' {
            self.pos += 1;
            self.parse_block(&mut node, depth, declared_array_len)?;
        }
        Ok(node)
    }

    /// Parses the comma-separated property list, returning the length declared
    /// by a `*N` array marker when one was present.
    fn parse_properties(&mut self, node: &mut FbxNode) -> io::Result<Option<usize>> {
        let mut declared_array_len = None;
        loop {
            self.skip_trivia();
            if self.pos >= self.input.len() {
                return Ok(declared_array_len);
            }
            match self.input[self.pos] {
                // Nothing follows the colon; the node has a block or is empty.
                b'{' | b'}' => return Ok(declared_array_len),
                b'*' => {
                    self.pos += 1;
                    let count = self.parse_number_text()?;
                    declared_array_len = count.parse::<usize>().ok();
                    if declared_array_len.is_none() {
                        return Err(self.error(format!("bad array length '{count}'")));
                    }
                }
                b'"' => {
                    let text = self.parse_string()?;
                    if is_base64_node(&node.name) {
                        if let Some(bytes) = decode_base64(&text) {
                            node.properties.push(FbxProperty::Raw(bytes));
                            continue;
                        }
                    }
                    node.properties.push(FbxProperty::String(text));
                }
                b',' => {
                    self.pos += 1;
                }
                byte if byte == b'-' || byte == b'+' || byte == b'.' || byte.is_ascii_digit() => {
                    let text = self.parse_number_text()?;
                    node.properties.push(number_property(&text));
                }
                // A bare word is either a boolean or the name of the next node,
                // in which case this property list ended at the newline. Only a
                // following ':' distinguishes them.
                byte if byte.is_ascii_alphabetic() => {
                    let save = self.pos;
                    let word = self.parse_identifier()?;
                    let mut probe = self.pos;
                    while probe < self.input.len()
                        && matches!(self.input[probe], b' ' | b'\t' | b'\r' | b'\n')
                    {
                        probe += 1;
                    }
                    if probe < self.input.len() && self.input[probe] == b':' {
                        self.pos = save;
                        return Ok(declared_array_len);
                    }
                    node.properties.push(match parse_ascii_bool(&word) {
                        Some(value) => FbxProperty::Bool(value),
                        None => FbxProperty::String(word),
                    });
                }
                _ => return Ok(declared_array_len),
            }
        }
    }

    fn parse_block(
        &mut self,
        node: &mut FbxNode,
        depth: u32,
        declared_array_len: Option<usize>,
    ) -> io::Result<()> {
        loop {
            self.skip_trivia();
            if self.pos >= self.input.len() {
                return Err(self.error("unterminated block"));
            }
            if self.input[self.pos] == b'}' {
                self.pos += 1;
                return Ok(());
            }
            let child = self.parse_node(depth + 1)?;
            // `Name: *N { a: ... }` is how ASCII writes an array property. The
            // values live in a child called `a`, which the binary container
            // does not have, so they are folded onto the parent as one array
            // property and the child is dropped.
            if child.name == "a" && declared_array_len.is_some() {
                let array = self.fold_array(&node.name, child, declared_array_len)?;
                node.properties.push(array);
            } else {
                node.children.push(child);
            }
        }
    }

    fn fold_array(
        &mut self,
        owner: &str,
        child: FbxNode,
        declared_array_len: Option<usize>,
    ) -> io::Result<FbxProperty> {
        let values = child.properties;
        if let Some(declared) = declared_array_len {
            if values.len() != declared {
                return Err(self.error(format!(
                    "array declares {declared} values but lists {}",
                    values.len()
                )));
            }
            if declared as u64 > self.options.limits.max_array_elements {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    format!(
                        "FBX: ASCII array of {declared} elements exceeds the {} element limit",
                        self.options.limits.max_array_elements
                    ),
                ));
            }
        }
        // Eight bytes per value is the widest binary form, so this bounds the
        // same quantity the binary reader's array budget does.
        self.array_bytes += values.len() as u64 * 8;
        if self.array_bytes > self.options.limits.max_total_array_raw_bytes {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "FBX: ASCII arrays exceed the {} byte total limit",
                    self.options.limits.max_total_array_raw_bytes
                ),
            ));
        }

        Ok(match array_element_type(owner) {
            ArrayElement::I32 => {
                FbxProperty::I32Array(values.iter().map(as_i64_value).map(|v| v as i32).collect())
            }
            ArrayElement::I64 => FbxProperty::I64Array(values.iter().map(as_i64_value).collect()),
            ArrayElement::F64 => FbxProperty::F64Array(values.iter().map(as_f64_value).collect()),
            ArrayElement::F32Bits => FbxProperty::F32Array(
                values
                    .iter()
                    .map(|value| match value {
                        // Written as an integer, so it is a bit pattern.
                        FbxProperty::I32(bits) => f32::from_bits(*bits as u32),
                        FbxProperty::I64(bits) => f32::from_bits(*bits as u32),
                        // Written with a decimal point, so it is a value.
                        other => as_f64_value(other) as f32,
                    })
                    .collect(),
            ),
        })
    }

    fn parse_identifier(&mut self) -> io::Result<String> {
        let start = self.pos;
        while self.pos < self.input.len() {
            let byte = self.input[self.pos];
            if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.error("expected a node name"));
        }
        Ok(String::from_utf8_lossy(&self.input[start..self.pos]).into_owned())
    }

    fn parse_number_text(&mut self) -> io::Result<String> {
        let start = self.pos;
        while self.pos < self.input.len() {
            let byte = self.input[self.pos];
            if byte.is_ascii_digit()
                || byte == b'.'
                || byte == b'-'
                || byte == b'+'
                || byte == b'e'
                || byte == b'E'
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.error("expected a number"));
        }
        Ok(String::from_utf8_lossy(&self.input[start..self.pos]).into_owned())
    }

    fn parse_string(&mut self) -> io::Result<String> {
        // The opening quote.
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos] != b'"' {
            self.pos += 1;
        }
        if self.pos >= self.input.len() {
            return Err(self.error("unterminated string"));
        }
        let raw = &self.input[start..self.pos];
        if raw.len() as u64 > self.options.limits.max_string_bytes {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "FBX: ASCII string of {} bytes exceeds the {} byte limit",
                    raw.len(),
                    self.options.limits.max_string_bytes
                ),
            ));
        }
        self.pos += 1;
        Ok(normalize_object_name(raw))
    }
}

fn as_i64_value(value: &FbxProperty) -> i64 {
    match value {
        FbxProperty::I32(v) => i64::from(*v),
        FbxProperty::I64(v) => *v,
        FbxProperty::F64(v) => *v as i64,
        FbxProperty::F32(v) => *v as i64,
        _ => 0,
    }
}

fn as_f64_value(value: &FbxProperty) -> f64 {
    match value {
        FbxProperty::I32(v) => f64::from(*v),
        FbxProperty::I64(v) => *v as f64,
        FbxProperty::F64(v) => *v,
        FbxProperty::F32(v) => f64::from(*v),
        _ => 0.0,
    }
}

/// Retypes a `Properties70` entry's values from the type it declares.
///
/// A `P` record names its own FBX type in the second field, so unlike a bare
/// array this needs no guessing: `P: "Lcl Scaling", "Lcl Scaling", "", "A",1,1,1`
/// is three doubles that merely happen to be written without a decimal point.
/// Left as integers they are invisible to every consumer that matches on
/// `F64`, which is how a scale of exactly 1 came to read as no scale at all.
fn coerce_typed_property(node: &mut FbxNode) {
    let Some(FbxProperty::String(type_name)) = node.properties.get(1) else {
        return;
    };
    if properties70_type_is_integral(type_name) {
        return;
    }
    for value in node.properties.iter_mut().skip(4) {
        if let FbxProperty::I32(number) = value {
            *value = FbxProperty::F64(f64::from(*number));
        } else if let FbxProperty::I64(number) = value {
            *value = FbxProperty::F64(*number as f64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<FbxNode> {
        parse_ascii_nodes(text.as_bytes(), &FbxReadOptions::default()).unwrap()
    }

    #[test]
    fn detects_both_header_spellings() {
        assert!(is_ascii_fbx(b"; FBX 7.5.0 project file\n"));
        assert!(is_ascii_fbx(b"FBXHeaderExtension:  {\n"));
        assert!(!is_ascii_fbx(b"Kaydara FBX Binary  \x00\x1a\x00"));
    }

    #[test]
    fn reads_nested_nodes_and_properties() {
        let nodes = parse(
            "Objects:  {\n\tModel: 42, \"Model::Cube\", \"Mesh\" {\n\t\tVersion: 232\n\t}\n}\n",
        );
        assert_eq!(nodes.len(), 1);
        let model = &nodes[0].children[0];
        assert_eq!(model.name, "Model");
        assert_eq!(model.properties.len(), 3);
        assert_eq!(model.children[0].name, "Version");
    }

    /// The name and class are reversed relative to the binary container, and
    /// everything above the node tree reads the binary spelling.
    #[test]
    fn object_names_are_rewritten_into_the_binary_spelling() {
        let nodes = parse("Objects:  {\n\tModel: 1, \"Model::Cube\", \"Mesh\" {\n\t}\n}\n");
        let model = &nodes[0].children[0];
        match &model.properties[1] {
            FbxProperty::String(name) => {
                assert_eq!(name, "Cube\u{0}\u{1}Model");
                assert_eq!(name.split('\0').next(), Some("Cube"));
            }
            other => panic!("expected a string, got {other:?}"),
        }
    }

    /// A whole-number `Vertices` array must not become an integer array: the
    /// geometry reader looks for floats, so guessing would drop the mesh.
    #[test]
    fn a_whole_number_vertex_array_still_reads_as_floats() {
        let nodes = parse("Geometry:  {\n\tVertices: *6 {\n\t\ta: -10,-10,0,10,-10,0\n\t}\n}\n");
        let vertices = &nodes[0].children[0];
        match &vertices.properties[0] {
            FbxProperty::F64Array(values) => {
                assert_eq!(values, &[-10.0, -10.0, 0.0, 10.0, -10.0, 0.0]);
            }
            other => panic!("expected an F64Array, got {other:?}"),
        }
    }

    #[test]
    fn an_index_array_reads_as_integers() {
        let nodes = parse("Geometry:  {\n\tPolygonVertexIndex: *4 {\n\t\ta: 0,1,3,-3\n\t}\n}\n");
        match &nodes[0].children[0].properties[0] {
            FbxProperty::I32Array(values) => assert_eq!(values, &[0, 1, 3, -3]),
            other => panic!("expected an I32Array, got {other:?}"),
        }
    }

    #[test]
    fn a_declared_length_that_disagrees_is_refused() {
        let error = parse_ascii_nodes(
            b"Geometry:  {\n\tVertices: *5 {\n\t\ta: 1.0,2.0\n\t}\n}\n",
            &FbxReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let nodes = parse("; a comment\n\nDefinitions:  {\n\t; another\n\tCount: 4\n}\n");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].children[0].name, "Count");
    }

    #[test]
    fn a_pre_6100_document_is_refused() {
        let error = parse_ascii_nodes(
            b"FBXHeaderExtension:  {\n\tFBXVersion: 5800\n}\n",
            &FbxReadOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("6100"), "{error}");
    }

    /// The 6100 name-keyed object model is read; only the layouts before it
    /// are refused.
    #[test]
    fn a_6100_document_parses() {
        let nodes = parse("FBXHeaderExtension:  {\n\tFBXVersion: 6100\n}\n");
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn nesting_beyond_the_limit_is_refused_without_overflowing_the_stack() {
        let mut text = String::new();
        for _ in 0..500 {
            text.push_str("N:  {\n");
        }
        for _ in 0..500 {
            text.push_str("}\n");
        }
        let error = parse_ascii_nodes(text.as_bytes(), &FbxReadOptions::default()).unwrap_err();
        assert!(error.to_string().contains("nesting"), "{error}");
    }
}

/// Regressions for defects the ASCII container exposed in code shared with the
/// binary reader.
///
/// These live here because ASCII is the only container that reaches them: the
/// binary form tags every value's width, so a consumer that matched one width
/// looked correct. They are written as inline documents rather than corpus
/// files so they run in CI, where `DRACO_FBX_CORPUS` is not set.
#[cfg(all(test, feature = "fbx-reader"))]
mod shared_path_regressions {
    use crate::FbxScene;

    fn scene(text: &str) -> FbxScene {
        FbxScene::from_bytes(text.as_bytes()).expect("document should decode")
    }

    /// Object ids are `i64` in the binary container but a bare number in ASCII,
    /// so an id small enough for `i32` arrives as one. Matching only `I64`
    /// skipped every object and returned an empty scene with nothing to
    /// explain it.
    #[test]
    fn objects_with_i32_sized_ids_are_still_indexed() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tGeometry: 100, \"Geometry::Tri\", \"Mesh\" {\n\
             \t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\
             \t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\
             \tModel: 200, \"Model::Cube\", \"Mesh\" {\n\t}\n}\n\
             Connections:  {\n\tC: \"OO\",100,200\n\tC: \"OO\",200,0\n}\n",
        );
        assert_eq!(decoded.root_nodes.len(), 1);
        assert_eq!(decoded.root_nodes[0].name.as_deref(), Some("Cube"));
        assert_eq!(decoded.root_nodes[0].mesh_instances.len(), 1);
        assert_eq!(
            decoded.root_nodes[0].mesh_instances[0].control_points.len(),
            3
        );
    }

    /// `bool::then_some` evaluates its argument eagerly, so reading a
    /// three-component property indexed the values before checking there were
    /// three of them. A `Lcl Scaling` with fewer panicked.
    #[test]
    fn a_short_vector_property_does_not_panic() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tModel: 200, \"Model::Cube\", \"Mesh\" {\n\
             \t\tProperties70:  {\n\
             \t\t\tP: \"InheritType\", \"enum\", \"\", \"\",3\n\
             \t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",2\n\t\t}\n\t}\n}\n\
             Connections:  {\n\tC: \"OO\",200,0\n}\n",
        );
        assert_eq!(decoded.root_nodes.len(), 1);
    }

    /// A document may connect a Model to one of its own ancestors. Following
    /// that cycle recursed until the stack was exhausted; the hierarchy now
    /// stops at the repeat.
    ///
    /// The cycle has to be reachable from a root to recurse at all: two models
    /// that are only each other's parent are both excluded from the top level,
    /// so nothing ever descends into them. The Root model's connection to the
    /// document root is what makes `A -> B -> A` reachable, and without it
    /// this test passes even with the guard removed.
    #[test]
    fn a_parent_cycle_terminates_instead_of_exhausting_the_stack() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tModel: 199, \"Model::Root\", \"Null\" {\n\t}\n\
             \tModel: 200, \"Model::A\", \"Null\" {\n\t}\n\
             \tModel: 201, \"Model::B\", \"Null\" {\n\t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",199,0\n\
             \tC: \"OO\",200,199\n\
             \tC: \"OO\",201,200\n\
             \tC: \"OO\",200,201\n}\n",
        );
        fn depth(node: &crate::FbxSceneNode) -> usize {
            1 + node.children.iter().map(depth).max().unwrap_or(0)
        }
        let deepest = decoded.root_nodes.iter().map(depth).max().unwrap_or(0);
        assert!(deepest <= 4, "cycle produced a chain {deepest} deep");
    }

    /// The same bound stops a chain that is deep rather than circular, and it
    /// used to stop it in silence. What the caller saw instead was the
    /// consequence: a 340-bone rig came back missing its tail, and every skin
    /// cluster bound into the missing part reported its own missing joint --
    /// 1116 notices, none of them naming the cause.
    #[test]
    fn a_hierarchy_deeper_than_the_reader_descends_says_so() {
        const BONES: usize = 300;
        let mut objects = String::new();
        let mut connections = String::from("\tC: \"OO\",1000,0\n");
        for index in 0..BONES {
            let id = 1000 + index;
            objects.push_str(&format!(
                "\tModel: {id}, \"Model::Bone{index:03}\", \"LimbNode\" {{\n\t}}\n"
            ));
            if index > 0 {
                connections.push_str(&format!("\tC: \"OO\",{id},{}\n", id - 1));
            }
        }
        let decoded = scene(&format!(
            "FBXHeaderExtension:  {{\n\tFBXVersion: 7500\n}}\n\
             Objects:  {{\n{objects}}}\n\
             Connections:  {{\n{connections}}}\n"
        ));

        fn depth(node: &crate::FbxSceneNode) -> usize {
            1 + node.children.iter().map(depth).max().unwrap_or(0)
        }
        let deepest = decoded.root_nodes.iter().map(depth).max().unwrap_or(0);
        assert!(
            deepest < BONES,
            "the chain was expected to be cut, not kept"
        );
        assert!(
            decoded
                .warnings
                .iter()
                .any(|warning| warning.code
                    == crate::fbx_scene::FbxWarningCode::ModelDepthLimitReached),
            "the cut has to be reported: warnings were {:?}",
            decoded
                .warnings
                .iter()
                .map(|warning| warning.code)
                .collect::<Vec<_>>()
        );
    }

    /// A whole-valued double is written without a decimal point, so a
    /// `DeformPercent: 100` arrives as an integer. Matching only the floating
    /// point widths read it as a missing weight, and a blend shape came back
    /// with a default of zero instead of the authored one.
    ///
    /// This has to go through a real blend shape rather than a plain transform
    /// property: those are read by a helper that already accepted integers, so
    /// a transform test passes either way and pins nothing.
    #[test]
    fn a_whole_valued_double_property_is_not_read_as_missing() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tGeometry: 100, \"Geometry::Tri\", \"Mesh\" {\n\
             \t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\
             \t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\
             \tModel: 200, \"Model::Cube\", \"Mesh\" {\n\t}\n\
             \tDeformer: 300, \"Deformer::Blend\", \"BlendShape\" {\n\t}\n\
             \tDeformer: 301, \"SubDeformer::Chan\", \"BlendShapeChannel\" {\n\
             \t\tDeformPercent: 100\n\
             \t\tFullWeights: *1 {\n\t\t\ta: 100\n\t\t}\n\t}\n\
             \tGeometry: 400, \"Geometry::Shape\", \"Shape\" {\n\
             \t\tIndexes: *1 {\n\t\t\ta: 0\n\t\t}\n\
             \t\tVertices: *3 {\n\t\t\ta: 0,0,1\n\t\t}\n\t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",100,200\n\
             \tC: \"OO\",200,0\n\
             \tC: \"OO\",300,100\n\
             \tC: \"OO\",301,300\n\
             \tC: \"OO\",400,301\n}\n",
        );
        let targets = &decoded.root_nodes[0].mesh_instances[0].morph_targets;
        assert_eq!(targets.len(), 1, "the blend shape target should be read");
        assert_eq!(
            targets[0].default_weight, 100.0,
            "an integer-written DeformPercent must not read as a missing weight"
        );
    }

    /// A Model that reaches neither the document root nor a parent Model by
    /// object connection is not part of the scene graph, and is dropped with
    /// a warning rather than rooted by absence -- which resurrected objects
    /// the source kept out of the scene, MotionBuilder's Producer cameras
    /// being the case the corpus carries.
    #[test]
    fn a_model_reaching_no_root_is_dropped_with_a_warning() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tModel: 200, \"Model::kept\", \"Null\" {\n\t}\n\
             \tModel: 201, \"Model::child\", \"Null\" {\n\t}\n\
             \tModel: 202, \"Model::orphan\", \"Null\" {\n\t}\n\
             \tModel: 203, \"Model::orphan child\", \"Null\" {\n\t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",200,0\n\
             \tC: \"OO\",201,200\n\
             \tC: \"OO\",203,202\n}\n",
        );
        fn names(nodes: &[crate::FbxSceneNode]) -> Vec<&str> {
            nodes
                .iter()
                .flat_map(|node| {
                    std::iter::once(node.name.as_deref().unwrap_or("?"))
                        .chain(names(&node.children))
                })
                .collect()
        }
        assert_eq!(names(&decoded.root_nodes), vec!["kept", "child"]);
        assert!(
            !decoded.warnings.is_empty(),
            "dropping the orphans must be reported"
        );
        assert!(decoded.warnings.iter().any(|warning| warning.code
            == crate::fbx_scene::FbxWarningCode::UnconnectedModelDropped
            && warning.message.contains("2 FBX Models")));
    }

    /// The animation side names a morph target by its position in the flat
    /// target list. That position is decided where the list is built, over
    /// shapes; counting channels per deformer instead agrees with it only when
    /// every deformer carries exactly one single-shape channel and nothing is
    /// dropped. With two `BlendShape` deformers both channels were numbered 0,
    /// their curves collapsed into one animation group and one target lost its
    /// animation entirely -- and a rewrite wrote the weights back onto the
    /// wrong targets, making the damage permanent.
    #[test]
    fn morph_animation_targets_come_from_the_target_list_they_index() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tGeometry: 100, \"Geometry::Tri\", \"Mesh\" {\n\
             \t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\
             \t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\
             \tModel: 200, \"Model::Cube\", \"Mesh\" {\n\t}\n\
             \tDeformer: 300, \"Deformer::A\", \"BlendShape\" {\n\t}\n\
             \tDeformer: 301, \"Deformer::ChanA\", \"BlendShapeChannel\" {\n\
             \t\tDeformPercent: 0\n\
             \t\tFullWeights: *1 {\n\t\t\ta: 100\n\t\t}\n\t}\n\
             \tGeometry: 310, \"Geometry::ShapeA\", \"Shape\" {\n\
             \t\tIndexes: *1 {\n\t\t\ta: 0\n\t\t}\n\
             \t\tVertices: *3 {\n\t\t\ta: 0,0,1\n\t\t}\n\t}\n\
             \tDeformer: 302, \"Deformer::B\", \"BlendShape\" {\n\t}\n\
             \tDeformer: 303, \"Deformer::ChanB\", \"BlendShapeChannel\" {\n\
             \t\tDeformPercent: 0\n\
             \t\tFullWeights: *1 {\n\t\t\ta: 100\n\t\t}\n\t}\n\
             \tGeometry: 311, \"Geometry::ShapeB\", \"Shape\" {\n\
             \t\tIndexes: *1 {\n\t\t\ta: 1\n\t\t}\n\
             \t\tVertices: *3 {\n\t\t\ta: 0,0,-1\n\t\t}\n\t}\n\
             \tAnimationStack: 500, \"AnimStack::Take\", \"\" {\n\t}\n\
             \tAnimationLayer: 510, \"AnimLayer::BaseLayer\", \"\" {\n\t}\n\
             \tAnimationCurveNode: 520, \"AnimCurveNode::ChanA\", \"\" {\n\t}\n\
             \tAnimationCurveNode: 521, \"AnimCurveNode::ChanB\", \"\" {\n\t}\n\
             \tAnimationCurve: 530, \"AnimCurve::ChanA\", \"\" {\n\
             \t\tKeyTime: *2 {\n\t\t\ta: 0,46186158000\n\t\t}\n\
             \t\tKeyValueFloat: *2 {\n\t\t\ta: 0,100\n\t\t}\n\t}\n\
             \tAnimationCurve: 531, \"AnimCurve::ChanB\", \"\" {\n\
             \t\tKeyTime: *2 {\n\t\t\ta: 0,46186158000\n\t\t}\n\
             \t\tKeyValueFloat: *2 {\n\t\t\ta: 0,100\n\t\t}\n\t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",100,200\n\
             \tC: \"OO\",200,0\n\
             \tC: \"OO\",300,100\n\
             \tC: \"OO\",301,300\n\
             \tC: \"OO\",310,301\n\
             \tC: \"OO\",302,100\n\
             \tC: \"OO\",303,302\n\
             \tC: \"OO\",311,303\n\
             \tC: \"OO\",500,0\n\
             \tC: \"OO\",510,500\n\
             \tC: \"OO\",520,510\n\
             \tC: \"OO\",521,510\n\
             \tC: \"OP\",520,301,\"DeformPercent\"\n\
             \tC: \"OP\",521,303,\"DeformPercent\"\n\
             \tC: \"OP\",530,520,\"d|X\"\n\
             \tC: \"OP\",531,521,\"d|X\"\n}\n",
        );
        let weights: Vec<Option<u32>> = decoded.animations[0]
            .channels
            .iter()
            .filter(|channel| channel.path == crate::fbx_scene::FbxAnimChannelPath::MorphWeight)
            .map(|channel| channel.morph_target_index)
            .collect();
        assert_eq!(
            weights,
            vec![Some(0), Some(1)],
            "two deformers must number their targets apart, not both at 0"
        );
    }

    /// A skin cluster's `Indexes` and `Weights` pair by position. Filtering
    /// only the indices dropped a negative entry from the left side while the
    /// weights kept their positions, and a later length "fix" truncated the
    /// weights -- so every influence after the bad entry landed on the control
    /// point one place earlier. A vertex could be driven by a weight meant for
    /// a different one entirely.
    #[test]
    fn a_negative_cluster_index_takes_its_weight_with_it() {
        let decoded = scene(&skin_document(Some("0,-1,1"), Some("0.25,99,0.75")));
        let skin = decoded.root_nodes[0].mesh_instances[0]
            .skin
            .as_ref()
            .expect("the skin should be read");
        assert_eq!(skin.clusters.len(), 1);
        let cluster = &skin.clusters[0];
        assert_eq!(cluster.control_point_indices, vec![0, 1]);
        // 99 belongs to the negative entry and must not land on point 1.
        assert_eq!(cluster.weights, vec![0.25, 0.75]);
    }

    /// An `Indexes` array that yields nothing -- empty, or one the binary
    /// reader could not narrow to `i32` -- paired with weights passed every
    /// length check before: the cluster was accepted with zero influences and
    /// its bone moved nothing, in silence. A cluster with nothing readable to
    /// influence is refused. (The unreadable variant exists only in the binary
    /// container: ASCII narrows every `Indexes` value as it parses.)
    #[test]
    fn an_empty_indexes_array_refuses_the_cluster() {
        let decoded = scene(&skin_document(None, Some("0.25,0.75")));
        let skin = decoded.root_nodes[0].mesh_instances[0]
            .skin
            .as_ref()
            .expect("the skin should be read");
        assert!(
            skin.clusters.is_empty(),
            "a cluster with no readable influences must be refused, not accepted inert"
        );
    }

    /// A minimal skinned triangle: one joint, one cluster with the given
    /// `Indexes`/`Weights` arrays.
    fn skin_document(indexes: Option<&str>, weights: Option<&str>) -> String {
        let cluster_arrays = |name: &str, values: Option<&str>| match values {
            Some(values) => format!(
                "		{name}: *{} {{
			a: {values}
		}}
",
                values.split(',').count()
            ),
            None => String::new(),
        };
        let mut cluster = String::from(
            "	Deformer: 301, \"Deformer::Cluster\", \"Cluster\" {
",
        );
        cluster.push_str(&cluster_arrays("Indexes", indexes));
        cluster.push_str(&cluster_arrays("Weights", weights));
        cluster.push_str(
            "	}
}
",
        );
        format!(
            "FBXHeaderExtension:  {{
	FBXVersion: 7500
}}
             Objects:  {{
             	Geometry: 100, \"Geometry::Tri\", \"Mesh\" {{
             		Vertices: *9 {{
			a: 0,0,0,1,0,0,0,1,0
		}}
             		PolygonVertexIndex: *3 {{
			a: 0,1,-3
		}}
	}}
             	Model: 200, \"Model::Cube\", \"Mesh\" {{
	}}
             	Model: 201, \"Model::Bone\", \"LimbNode\" {{
	}}
             	Deformer: 300, \"Deformer::Skin\", \"Skin\" {{
	}}
{}             Connections:  {{
             	C: \"OO\",100,200
             	C: \"OO\",200,0
             	C: \"OO\",201,0
             	C: \"OO\",300,100
             	C: \"OO\",301,300
             	C: \"OO\",201,301
}}
",
            cluster
        )
    }

    /// An axis an `AnimationCurveNode` connects no curve to is not animated
    /// but not valueless: the exporter writes its constant into the node's
    /// own `Properties70` as `d|X`/`d|Y`/`d|Z`. Reading curves only left the
    /// static axes at zero -- a translation snapped to the origin and a scale
    /// collapsed the object. Blender's exporter writes these entries, so the
    /// data was there and went unread.
    #[test]
    fn a_static_axis_reads_its_curve_node_value() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tGeometry: 100, \"Geometry::Tri\", \"Mesh\" {\n\
             \t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\
             \t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\
             \tModel: 200, \"Model::Cube\", \"Mesh\" {\n\t}\n\
             \tAnimationStack: 500, \"AnimStack::Take\", \"\" {\n\t}\n\
             \tAnimationLayer: 510, \"AnimLayer::BaseLayer\", \"\" {\n\t}\n\
             \tAnimationCurveNode: 520, \"AnimCurveNode::T\", \"\" {\n\
             \t\tProperties70:  {\n\
             \t\t\tP: \"d|Y\", \"Number\", \"\", \"A\",2\n\
             \t\t\tP: \"d|Z\", \"Number\", \"\", \"A\",3\n\
             \t\t}\n\t}\n\
             \tAnimationCurve: 530, \"AnimCurve::X\", \"\" {\n\
             \t\tKeyTime: *2 {\n\t\t\ta: 0,46186158000\n\t\t}\n\
             \t\tKeyValueFloat: *2 {\n\t\t\ta: 0,10\n\t\t}\n\t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",100,200\n\
             \tC: \"OO\",200,0\n\
             \tC: \"OO\",500,0\n\
             \tC: \"OO\",510,500\n\
             \tC: \"OO\",520,510\n\
             \tC: \"OP\",520,200,\"Lcl Translation\"\n\
             \tC: \"OP\",530,520,\"d|X\"\n}\n",
        );
        let output = &decoded.animations[0].channels[0].sampler.output;
        assert_eq!(
            output,
            &[0.0, 2.0, 3.0, 10.0, 2.0, 3.0],
            "the static Y and Z must hold their stated values, not zero"
        );
    }

    /// And where the document states nothing at all -- no curve on the axis
    /// and no `d|*` entry beside it -- the axis holds the property's own
    /// default. For a scale that is one. Zero is the right answer for a
    /// rotation or a translation and the wrong one here: a file that animates
    /// a single axis of a scale would flatten the object on the other two.
    #[test]
    fn an_unstated_scale_axis_holds_one_rather_than_zero() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 7500\n}\n\
             Objects:  {\n\
             \tGeometry: 100, \"Geometry::Tri\", \"Mesh\" {\n\
             \t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\
             \t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\
             \tModel: 200, \"Model::Cube\", \"Mesh\" {\n\t}\n\
             \tAnimationStack: 500, \"AnimStack::Take\", \"\" {\n\t}\n\
             \tAnimationLayer: 510, \"AnimLayer::BaseLayer\", \"\" {\n\t}\n\
             \tAnimationCurveNode: 520, \"AnimCurveNode::S\", \"\" {\n\t}\n\
             \tAnimationCurve: 530, \"AnimCurve::X\", \"\" {\n\
             \t\tKeyTime: *2 {\n\t\t\ta: 0,46186158000\n\t\t}\n\
             \t\tKeyValueFloat: *2 {\n\t\t\ta: 1,4\n\t\t}\n\t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",100,200\n\
             \tC: \"OO\",200,0\n\
             \tC: \"OO\",500,0\n\
             \tC: \"OO\",510,500\n\
             \tC: \"OO\",520,510\n\
             \tC: \"OP\",520,200,\"Lcl Scaling\"\n\
             \tC: \"OP\",530,520,\"d|X\"\n}\n",
        );
        let output = &decoded.animations[0].channels[0].sampler.output;
        assert_eq!(
            output,
            &[1.0, 1.0, 1.0, 4.0, 1.0, 1.0],
            "the axes the file leaves unstated hold the scale default, not zero"
        );
    }

    /// The pre-7000 `Takes` section carries no statics of its own: an axis
    /// without a `Channel` subtree is constant at the Model's transform
    /// property. Reading only channels left such axes at zero.
    #[test]
    fn a_takes_channel_reads_its_model_property_for_missing_axes() {
        let decoded = scene(
            "FBXHeaderExtension:  {\n\tFBXVersion: 6100\n}\n\
             Objects:  {\n\
             \tGeometry: \"Geometry::Tri\", \"Mesh\" {\n\
             \t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\
             \t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\
             \tModel: \"Model::Cube\", \"Mesh\" {\n\
             \t\tProperties70:  {\n\
             \t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",0,2,3\n\
             \t\t}\n\t}\n}\n\
             Takes:  {\n\
             \tTake: \"Take 001\" {\n\
             \t\tModel: \"Model::Cube\" {\n\
             \t\t\tChannel: \"Transform\" {\n\
             \t\t\t\tChannel: \"T\" {\n\
             \t\t\t\t\tChannel: \"X\" {\n\
             \t\t\t\t\t\tDefault: 0\n\
             \t\t\t\t\t\tKeyVer: 4005\n\
             \t\t\t\t\t\tKeyCount: 2\n\
             \t\t\t\t\t\tKey: 0,0,L,46186158000,10,L\n\
             \t\t\t\t\t}\n\
             \t\t\t\t}\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n}\n\
             Connections:  {\n\
             \tC: \"OO\",\"Geometry::Tri\",\"Model::Cube\"\n\
             \tC: \"OO\",\"Model::Cube\",0\n}\n",
        );
        let output = &decoded.animations[0].channels[0].sampler.output;
        assert_eq!(
            output,
            &[0.0, 2.0, 3.0, 10.0, 2.0, 3.0],
            "Y and Z have no channels and must hold the model's translation"
        );
    }
}
