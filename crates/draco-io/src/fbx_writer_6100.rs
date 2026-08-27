//! The FBX 6100 spelling of a document.
//!
//! The writer builds one document -- the 7500 one -- and this module rewrites
//! it into the pre-7000 object model, rather than a second builder growing
//! beside the first. Everything either container prints afterwards is the same
//! node tree, so the two spellings cannot drift apart semantically any more
//! than the two containers can.
//!
//! What changes, object by object:
//!
//! * Objects are keyed by a `"Name\0\x01Class"` string instead of an `i64` id,
//!   and connections carry those strings. The scene root is
//!   `Scene\0\x01Model`, the id 0 of 7.x.
//! * `Geometry` does not exist as an object: its records move onto the `Model`
//!   that carried it.
//! * `Properties70` becomes `Properties60`, whose `Property` records lead with
//!   three strings rather than four.
//! * Array payloads become repeated scalar properties; the 7.x array record is
//!   a 7.x mechanism.
//! * Animation has no stack objects: channels move into the `Takes` section,
//!   one `Take` per stack, with the key data in the heterogeneous `Key` run
//!   the reader reverses.
//!
//! Skins and blend shapes have no 6100 spelling here: the deformer topology
//! the 7.x document writes has no pre-7000 counterpart this crate could round
//! trip, so asking for 6100 with one attached is an error rather than a file
//! that silently drops it.

use std::collections::HashMap;

use crate::fbx_node::{FbxNode, FbxProperty};
use crate::fbx_scene::FbxNodeId;
use crate::fbx_writer::AnimStackData;

/// The key that plays the role 7.x gives the id 0.
const ROOT_KEY: &str = "Scene\0\x01Model";

/// The FBX < 8000 KTime ticks per second, as the 7.x writer uses it.
const KTIME: f64 = 46_186_158_000.0;

/// Rewrites a 7500 document tree into the 6100 object model.
///
/// `node_model_ids` maps a scene node's stable id to the 7.x object id the
/// writer gave its `Model`, the same mapping the 7.x connection/skin/pose
/// writers already resolve animation and bind-pose targets through -- an
/// animation channel is otherwise only addressable by display name, which two
/// sibling nodes may share.
pub(crate) fn translate(
    document: &mut Vec<FbxNode>,
    anim: &[AnimStackData],
    node_model_ids: &HashMap<FbxNodeId, i64>,
) {
    // Keyed before anything is rewritten, because the id properties the table
    // is built from are the first thing the translation replaces.
    let keys = object_keys_of(document);
    let geometry_owners = geometry_owners_of(document);
    for node in document.iter_mut() {
        match node.name.as_str() {
            "FBXHeaderExtension" => set_version(node, 6100),
            "GlobalSettings" => to_properties60(node),
            "Definitions" => prune_definitions(node),
            "Objects" => translate_objects(node, &keys, &geometry_owners),
            "Connections" => translate_connections(node, &keys),
            _ => {}
        }
    }
    if let Some(takes) = takes_node(anim, &keys, node_model_ids) {
        document.push(takes);
    }
}

/// Assigns every object in `Objects` its unique name key, by id.
fn object_keys(objects: &FbxNode) -> HashMap<i64, String> {
    let mut keys = HashMap::new();
    // Keys live in one namespace per class: a Model and a Material may share a
    // name, two Models may not. Pre-seeding the root's own key here means an
    // ordinary Model actually named "Scene" collides with it the same way two
    // same-named Models collide with each other, rather than aliasing the
    // document root on read-back.
    let mut used: HashMap<String, usize> = HashMap::new();
    used.insert(ROOT_KEY.to_string(), 0);
    for child in &objects.children {
        let Some(FbxProperty::I64(id)) = child.properties.first() else {
            continue;
        };
        // The key string spells both halves: Name, separator, Class. The
        // class property beside it names the subclass -- `Mesh` on a Model --
        // and is not the class the key is built from.
        let Some(FbxProperty::String(raw)) = child.properties.get(1) else {
            continue;
        };
        let Some((name, class)) = raw.split_once('\0') else {
            continue;
        };
        let name = name.to_string();
        let class = class.trim_start_matches('').to_string();
        if matches!(
            child.name.as_str(),
            "Geometry"
                | "AnimationStack"
                | "AnimationLayer"
                | "AnimationCurveNode"
                | "AnimationCurve"
                | "Deformer"
                | "Pose"
        ) {
            // These objects do not survive the translation; their ids never
            // resolve, which is how their connections are dropped.
            continue;
        }
        let base = format!("{name}\0\x01{class}");
        let unique = match used.get_mut(&base) {
            None => {
                used.insert(base.clone(), 0);
                base
            }
            Some(count) => {
                *count += 1;
                format!("{base}#{count}")
            }
        };
        keys.insert(*id, unique);
    }
    keys
}

/// Each geometry object's owning Model, read from the connection edges
/// before the rewrite replaces the ids they are stated in.
fn geometry_owners_of(document: &[FbxNode]) -> HashMap<i64, i64> {
    let mut owners = HashMap::new();
    let Some(connections) = document.iter().find(|node| node.name == "Connections") else {
        return owners;
    };
    let geometries = document
        .iter()
        .find(|node| node.name == "Objects")
        .map(|node| {
            node.children
                .iter()
                .filter(|child| child.name == "Geometry")
                .filter_map(|child| match child.properties.first() {
                    Some(FbxProperty::I64(id)) => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for child in &connections.children {
        if child.name != "C" {
            continue;
        }
        let (Some(child_id), Some(parent_id)) = (
            child.properties.get(1).and_then(id_of),
            child.properties.get(2).and_then(id_of),
        ) else {
            continue;
        };
        if geometries.contains(&child_id) {
            owners.insert(child_id, parent_id);
        }
    }
    owners
}

/// The same table, found from the document root.
fn object_keys_of(document: &[FbxNode]) -> HashMap<i64, String> {
    document
        .iter()
        .find(|node| node.name == "Objects")
        .map(object_keys)
        .unwrap_or_default()
}

fn set_version(header: &mut FbxNode, version: u32) {
    for child in &mut header.children {
        if child.name == "FBXVersion" {
            child.properties = vec![FbxProperty::I32(version as i32)];
        }
    }
}

/// Rewrites a `Properties70` block (and its `P` records) into `Properties60`.
fn to_properties60(holder: &mut FbxNode) {
    for block in holder.children.iter_mut() {
        if block.name == "Properties70" {
            block.name = "Properties60".to_string();
            for record in &mut block.children {
                if record.name == "P" {
                    record.name = "Property".to_string();
                    // `P` leads with name, type, subtype, flags; `Property`
                    // has no subtype.
                    if record.properties.len() > 3 {
                        record.properties.remove(2);
                    }
                }
            }
        }
    }
}

/// Drops the `ObjectType` declarations of objects the 6100 model does not
/// carry, and recounts the block list.
fn prune_definitions(definitions: &mut FbxNode) {
    const DROPPED: [&str; 6] = [
        "Geometry",
        "AnimationStack",
        "AnimationLayer",
        "AnimationCurveNode",
        "AnimationCurve",
        "Deformer",
    ];
    definitions
        .children
        .retain(|child| !(child.name == "ObjectType" && matches!(child.properties.first(), Some(FbxProperty::String(t)) if DROPPED.contains(&t.as_str()))));
    let count = definitions
        .children
        .iter()
        .filter(|child| child.name == "ObjectType")
        .count();
    for child in &mut definitions.children {
        if child.name == "Count" {
            child.properties = vec![FbxProperty::I32(count as i32)];
        }
    }
}

/// Rewrites `Objects`: name keys in place of ids, geometry folded onto its
/// Model, properties and arrays in the 6100 spelling.
fn translate_objects(
    objects: &mut FbxNode,
    keys: &HashMap<i64, String>,
    geometry_owners: &HashMap<i64, i64>,
) {
    // The geometry records move onto their Model, found by the connection
    // that joined them; that edge is dropped when the connections are
    // rewritten, so the pairing is made here first.
    let mut geometry_by_model: HashMap<i64, Vec<FbxNode>> = HashMap::new();
    objects.children.retain(|child| {
        if child.name != "Geometry" {
            return true;
        }
        if let Some(FbxProperty::I64(id)) = child.properties.first() {
            if let Some(owner) = geometry_owners.get(id) {
                geometry_by_model.insert(*owner, child.children.clone());
            }
        }
        false
    });

    for child in objects.children.iter_mut() {
        let Some(FbxProperty::I64(id)) = child.properties.first() else {
            continue;
        };
        let id = *id;
        match child.name.as_str() {
            "Model" => {
                rekey_object(child, id, keys);
                if let Some(mut records) = geometry_by_model.remove(&id) {
                    for record in &mut records {
                        scalars_only(record);
                    }
                    child.children.extend(records);
                }
            }
            "AnimationStack" | "AnimationLayer" | "AnimationCurveNode" | "AnimationCurve" => {}
            "Material" | "Texture" | "Video" | "NodeAttribute" => rekey_object(child, id, keys),
            _ => {}
        }
    }
    objects.children.retain(|child| {
        !matches!(
            child.name.as_str(),
            "AnimationStack" | "AnimationLayer" | "AnimationCurveNode" | "AnimationCurve"
        )
    });
}

/// Rewrites an object's own `[id, "Name\0\x01Class", class]` triple into the
/// 6100 `[key, class]` pair, and its `Properties70` into `Properties60`.
fn rekey_object(child: &mut FbxNode, id: i64, keys: &HashMap<i64, String>) {
    let class = match child.properties.get(2) {
        Some(FbxProperty::String(class)) => class.clone(),
        _ => String::new(),
    };
    if let Some(key) = keys.get(&id) {
        child.properties = vec![FbxProperty::String(key.clone()), FbxProperty::String(class)];
    }
    to_properties60(child);
}

/// Replaces array properties with their elements, spelled one property each.
fn scalars_only(node: &mut FbxNode) {
    let mut scalars = Vec::with_capacity(node.properties.len());
    for property in node.properties.drain(..) {
        match property {
            FbxProperty::BoolArray(values) => {
                scalars.extend(values.into_iter().map(FbxProperty::Bool))
            }
            FbxProperty::I32Array(values) => {
                scalars.extend(values.into_iter().map(FbxProperty::I32))
            }
            FbxProperty::I64Array(values) => {
                scalars.extend(values.into_iter().map(FbxProperty::I64))
            }
            FbxProperty::F32Array(values) => {
                scalars.extend(values.into_iter().map(FbxProperty::F32))
            }
            FbxProperty::F64Array(values) => {
                scalars.extend(values.into_iter().map(FbxProperty::F64))
            }
            other => scalars.push(other),
        }
    }
    node.properties = scalars;
    for child in node.children.iter_mut() {
        scalars_only(child);
    }
}

/// Rewrites `Connections` around the name keys.
fn translate_connections(connections: &mut FbxNode, keys: &HashMap<i64, String>) {
    let mut edges = Vec::new();
    for child in connections.children.drain(..) {
        if child.name != "C" {
            continue;
        }
        let mut properties = child.properties.into_iter();
        let Some(kind) = properties.next() else {
            continue;
        };
        let (Some(child_id), Some(parent_id)) = (
            properties.next().as_ref().and_then(id_of),
            properties.next().as_ref().and_then(id_of),
        ) else {
            continue;
        };
        let property = properties.next();
        // Edges to objects the 6100 model does not carry -- a merged-in
        // geometry, an animation stack -- resolve to nothing and drop.
        if let (Some(child_key), Some(parent_key)) =
            (keys.get(&child_id), resolve_parent(keys, parent_id))
        {
            let mut edge = vec![
                kind,
                FbxProperty::String(child_key.clone()),
                FbxProperty::String(parent_key.clone()),
            ];
            if let Some(property) = property {
                edge.push(property);
            }
            edges.push(FbxNode {
                name: "Connect".to_string(),
                properties: edge,
                children: Vec::new(),
            });
        }
    }
    connections.children = edges;
}

fn id_of(property: &FbxProperty) -> Option<i64> {
    match property {
        FbxProperty::I64(id) => Some(*id),
        FbxProperty::I32(id) => Some(i64::from(*id)),
        _ => None,
    }
}

fn resolve_parent(keys: &HashMap<i64, String>, parent: i64) -> Option<String> {
    if parent == 0 {
        return Some(ROOT_KEY.to_string());
    }
    keys.get(&parent).cloned()
}

/// Builds the `Takes` section from the stacks the 7.x document would have
/// written as animation objects.
fn takes_node(
    anim: &[AnimStackData],
    keys: &HashMap<i64, String>,
    node_model_ids: &HashMap<FbxNodeId, i64>,
) -> Option<FbxNode> {
    if anim.is_empty() {
        return None;
    }
    let mut takes = FbxNode {
        name: "Takes".to_string(),
        properties: Vec::new(),
        children: Vec::new(),
    };
    for stack in anim {
        let name = stack.name.clone().unwrap_or_else(|| "Take".to_string());
        if takes.children.is_empty() {
            takes.children.push(FbxNode {
                name: "Current".to_string(),
                properties: vec![FbxProperty::String(name.clone())],
                children: Vec::new(),
            });
        }
        let mut take = FbxNode {
            name: "Take".to_string(),
            properties: vec![FbxProperty::String(name)],
            children: Vec::new(),
        };
        for channel in &stack.channels {
            let Some(model_key) = model_key_for(channel, keys, node_model_ids) else {
                continue;
            };
            let model = take
                .children
                .iter_mut()
                .find(|child| {
                    matches!(&child.properties.first(), Some(FbxProperty::String(key)) if key == &model_key)
                });
            let model = match model {
                Some(model) => model,
                None => {
                    take.children.push(FbxNode {
                        name: "Model".to_string(),
                        properties: vec![FbxProperty::String(model_key)],
                        children: vec![FbxNode {
                            name: "Version".to_string(),
                            properties: vec![FbxProperty::F64(1.1)],
                            children: Vec::new(),
                        }],
                    });
                    take.children.last_mut().expect("just pushed")
                }
            };
            let group_name = match channel.path {
                crate::fbx_scene::FbxAnimChannelPath::Translation => "T",
                crate::fbx_scene::FbxAnimChannelPath::Rotation => "R",
                crate::fbx_scene::FbxAnimChannelPath::Scale => "S",
                _ => continue,
            };
            let transform = ensure_child(model, "Channel", "Transform");
            let group = ensure_child(transform, "Channel", group_name);
            for component in 0..3usize {
                let axis = ["X", "Y", "Z"][component];
                let values: Vec<f32> = channel
                    .sampler
                    .output
                    .iter()
                    .skip(component)
                    .step_by(channel.path.component_count())
                    .copied()
                    .collect();
                if values.is_empty() {
                    continue;
                }
                let channel_node = FbxNode {
                    name: "Channel".to_string(),
                    properties: vec![FbxProperty::String(axis.to_string())],
                    children: vec![
                        FbxNode {
                            name: "Default".to_string(),
                            properties: vec![FbxProperty::F64(f64::from(
                                values.first().copied().unwrap_or(0.0),
                            ))],
                            children: Vec::new(),
                        },
                        FbxNode {
                            name: "KeyVer".to_string(),
                            properties: vec![FbxProperty::I32(4005)],
                            children: Vec::new(),
                        },
                        FbxNode {
                            name: "KeyCount".to_string(),
                            properties: vec![FbxProperty::I32(values.len() as i32)],
                            children: Vec::new(),
                        },
                        key_node(
                            &channel.sampler.input,
                            &values,
                            &channel.sampler,
                            channel.path,
                        ),
                    ],
                };
                group.children.push(channel_node);
            }
        }
        takes.children.push(take);
    }
    Some(takes)
}

/// The model key an animated channel belongs to.
///
/// `FbxAnimChannel::node_id` is documented as the stable target -- never
/// resolve a channel by display name, since two sibling nodes may share one --
/// so this goes through `node_model_ids` (scene id to 7.x object id) and then
/// `keys` (7.x object id to its 6100 key) rather than matching `node_name`
/// against the key table.
fn model_key_for(
    channel: &crate::fbx_scene::FbxAnimChannel,
    keys: &HashMap<i64, String>,
    node_model_ids: &HashMap<FbxNodeId, i64>,
) -> Option<String> {
    let model_id = node_model_ids.get(&channel.node_id)?;
    keys.get(model_id).cloned()
}

/// A `Key` record: the heterogeneous run the reader walks with its state
/// machine, spelled as one property per field.
///
/// FBX stores Euler angles in degrees while the sampler carries radians --
/// the same conversion the 7.x curve writer makes, applied to the slopes too
/// because they are in output units per second.
///
/// Each key states its own right (out) slope and the left (in) slope the
/// *next* key inherits, the shape the format's `s` mode carries -- there is
/// no field for a key's own left slope, so the first key's `in_tangent` has
/// no 6100 spelling and is not written; a cubic curve authored with a
/// meaningful incoming slope on its first key loses that value on a 6100
/// round trip. Read back, the reader's own first key seeds it at zero for the
/// same reason.
fn key_node(
    times: &[f32],
    values: &[f32],
    sampler: &crate::fbx_scene::FbxAnimSampler,
    path: crate::fbx_scene::FbxAnimChannelPath,
) -> FbxNode {
    let unit = if path == crate::fbx_scene::FbxAnimChannelPath::Rotation {
        180.0 / std::f64::consts::PI
    } else {
        1.0
    };
    let unit = |value: f32| f64::from(value) * unit;
    use crate::fbx_scene::FbxAnimInterpolation;
    let mut properties = Vec::new();
    for (index, (&time, &value)) in times.iter().zip(values).enumerate() {
        properties.push(FbxProperty::I64((f64::from(time) * KTIME) as i64));
        properties.push(FbxProperty::F64(unit(value)));
        match sampler.interpolation {
            FbxAnimInterpolation::Linear => properties.push(mode('L')),
            FbxAnimInterpolation::Step => {
                properties.push(mode('C'));
                properties.push(mode('n'));
            }
            FbxAnimInterpolation::Cubic => {
                properties.push(mode('U'));
                properties.push(mode('s'));
                // Right slope of this key, then the left slope the next one
                // inherits -- the two fields the `s` mode carries. Tangents
                // are in output units per second, which is the unit a slope
                // is.
                let right = sampler
                    .out_tangents
                    .as_ref()
                    .and_then(|tangents| tangents.get(index))
                    .copied()
                    .unwrap_or(0.0);
                let next_left = sampler
                    .in_tangents
                    .as_ref()
                    .and_then(|tangents| tangents.get(index + 1))
                    .copied()
                    .unwrap_or(0.0);
                properties.push(FbxProperty::F64(unit(right)));
                properties.push(FbxProperty::F64(unit(next_left)));
                properties.push(mode('n'));
            }
        }
    }
    FbxNode {
        name: "Key".to_string(),
        properties,
        children: Vec::new(),
    }
}

fn mode(letter: char) -> FbxProperty {
    // A one-character string, in either container: the binary `C` record and
    // the ASCII bare word both read back as the mode letter the reader
    // expects, and neither loses the byte.
    FbxProperty::String(letter.to_string())
}

/// Finds or creates the child `Channel` node with this name.
fn ensure_child<'a>(parent: &'a mut FbxNode, name: &str, channel: &str) -> &'a mut FbxNode {
    let index = parent
        .children
        .iter()
        .position(|child| {
            child.name == name
                && matches!(&child.properties.first(), Some(FbxProperty::String(p)) if p == channel)
        })
        .unwrap_or_else(|| {
            parent.children.push(FbxNode {
                name: name.to_string(),
                properties: vec![FbxProperty::String(channel.to_string())],
                children: Vec::new(),
            });
            parent.children.len() - 1
        });
    &mut parent.children[index]
}
