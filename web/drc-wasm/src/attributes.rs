//! Attributes by id or by type, in the typed array a caller asks for.
//!
//! `parse_drc_bytes` reads a mesh the way the converter shows it: positions,
//! normals, texture coordinates and colours by type, as floats. A glTF loader
//! needs something else. `KHR_draco_mesh_compression` names each glTF
//! attribute's Draco attribute by unique id, and the accessor says which
//! component type the values must come back in. Upstream's JavaScript decoder
//! answers that with `GetAttributeByUniqueId` and
//! `GetAttributeDataArrayForAllPoints`, which three.js's `DRACOLoader` drives,
//! and this is the same answer: the same attributes, converted by the same rule,
//! byte for byte.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{AttributeValueIndex, PointIndex};
use draco_core::mesh::Mesh;
use js_sys::{
    Array, BigInt64Array, BigUint64Array, Float32Array, Float64Array, Int16Array, Int32Array,
    Int8Array, Object, Reflect, Uint16Array, Uint32Array, Uint8Array,
};
use wasm_bindgen::prelude::*;
use wasm_bridge::{set_bool, set_js, set_opt_string, set_string_array};

/// Decodes a Draco stream and returns its attributes as typed arrays.
///
/// `attributes` is `undefined` for every attribute in its own component type,
/// or an array of requests `{ name, id, semantic, type }`:
///
/// - `id`, a unique id, finds that attribute, as glTF's
///   `KHR_draco_mesh_compression` names them; a missing one is an error.
/// - `semantic` (`"POSITION"`, `"NORMAL"`, `"COLOR"`, `"TEX_COORD"` or
///   `"GENERIC"`) finds the first attribute of that type instead, as a `.drc`
///   file is read; a missing one is left out.
/// - `type`, a typed array's name such as `"Float32Array"` or
///   `"Uint16Array"`, converts the values the way upstream's
///   `GetAttributeDataArrayForAllPoints` does; without it the values keep
///   their own type. A value the type cannot hold is an error.
///
/// The result is `{ success, error, warnings, geometry, index, attributes }`,
/// where `geometry` is `"mesh"` or `"point_cloud"` as the stream's header says,
/// `index` a `Uint32Array` of triangle corners for a mesh and `null` for a
/// point cloud, and each attribute `{ name, uniqueId, semantic, array,
/// itemSize, normalized }`.
#[wasm_bindgen]
pub fn decode_draco(data: &[u8], attributes: JsValue) -> JsValue {
    match decode(data, &attributes) {
        Ok(result) => result.into(),
        Err(error) => {
            let obj = Object::new();
            set_bool(&obj, "success", false);
            set_opt_string(&obj, "error", &Some(error));
            set_string_array(&obj, "warnings", &[]);
            set_js(&obj, "attributes", &Array::new().into());
            set_js(&obj, "index", &JsValue::NULL);
            obj.into()
        }
    }
}

/// What a request names, and how its values should come back.
struct Request {
    name: Option<String>,
    find: Find,
    target: Option<Target>,
}

enum Find {
    UniqueId(u32),
    Semantic(GeometryAttributeType),
}

/// The typed arrays a request may ask for: the ones three.js's `DRACOLoader`
/// maps to Draco data types, and so the ones upstream's decoder converts to.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
}

fn decode(data: &[u8], attributes: &JsValue) -> Result<Object, String> {
    let requests = parse_requests(attributes)?;
    let mesh = crate::decode_mesh(data)?;
    // Byte 7 of the header is the encoder type, which upstream's
    // `GetEncodedGeometryType` reads: 0 a point cloud, 1 a triangular mesh.
    let is_mesh = data.get(7) == Some(&1);

    let out = Array::new();
    match requests {
        None => {
            for id in 0..mesh.num_attributes() {
                let attribute = mesh.attribute(id);
                let array = convert(&mesh, attribute, None)?;
                out.push(&attribute_to_js(&mesh, attribute, None, array).into());
            }
        }
        Some(requests) => {
            for request in &requests {
                let attribute = match request.find {
                    Find::UniqueId(unique_id) => mesh
                        .attribute_by_unique_id(unique_id)
                        .ok_or_else(|| format!("no attribute has unique id {unique_id}"))?,
                    Find::Semantic(attribute_type) => {
                        let id = mesh.named_attribute_id(attribute_type);
                        if id < 0 {
                            continue;
                        }
                        mesh.attribute(id)
                    }
                };
                let array = convert(&mesh, attribute, request.target)
                    .map_err(|error| format!("{}: {error}", label(request, attribute)))?;
                out.push(&attribute_to_js(&mesh, attribute, request.name.as_deref(), array).into());
            }
        }
    }

    let obj = Object::new();
    set_bool(&obj, "success", true);
    set_opt_string(&obj, "error", &None);
    set_string_array(&obj, "warnings", &[]);
    set_js(
        &obj,
        "geometry",
        &JsValue::from_str(if is_mesh { "mesh" } else { "point_cloud" }),
    );
    let index = if is_mesh {
        let mut corners = Vec::with_capacity(mesh.num_faces() * 3);
        for face in 0..mesh.num_faces() {
            let face = mesh.face(draco_core::geometry_indices::FaceIndex(face as u32));
            corners.extend(face.iter().map(|point| point.0));
        }
        Uint32Array::from(corners.as_slice()).into()
    } else {
        JsValue::NULL
    };
    set_js(&obj, "index", &index);
    set_js(&obj, "attributes", &out.into());
    Ok(obj)
}

fn label(request: &Request, attribute: &PointAttribute) -> String {
    match &request.name {
        Some(name) => format!("attribute {name} (unique id {})", attribute.unique_id()),
        None => format!("attribute with unique id {}", attribute.unique_id()),
    }
}

fn parse_requests(value: &JsValue) -> Result<Option<Vec<Request>>, String> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let array: &Array = value
        .dyn_ref()
        .ok_or("attributes must be an array of requests or undefined")?;
    let field = |entry: &JsValue, key: &str| Reflect::get(entry, &JsValue::from_str(key)).ok();
    let mut requests = Vec::with_capacity(array.length() as usize);
    for (position, entry) in array.iter().enumerate() {
        let name = field(&entry, "name").and_then(|value| value.as_string());
        let id = field(&entry, "id").filter(|value| !value.is_undefined() && !value.is_null());
        let semantic = field(&entry, "semantic").and_then(|value| value.as_string());
        let find = match (id, semantic) {
            (Some(id), _) => {
                let id = id
                    .as_f64()
                    .filter(|id| id.fract() == 0.0 && *id >= 0.0 && *id <= f64::from(u32::MAX))
                    .ok_or_else(|| {
                        format!("request {position}: id must be a non-negative integer")
                    })?;
                Find::UniqueId(id as u32)
            }
            (None, Some(semantic)) => Find::Semantic(
                semantic_from_name(&semantic)
                    .ok_or_else(|| format!("request {position}: unknown semantic {semantic}"))?,
            ),
            (None, None) => {
                return Err(format!(
                    "request {position} names neither an id nor a semantic"
                ))
            }
        };
        let target = match field(&entry, "type").and_then(|value| value.as_string()) {
            None => None,
            Some(name) => Some(
                target_from_name(&name)
                    .ok_or_else(|| format!("request {position}: unsupported array type {name}"))?,
            ),
        };
        requests.push(Request { name, find, target });
    }
    Ok(Some(requests))
}

fn semantic_from_name(name: &str) -> Option<GeometryAttributeType> {
    Some(match name {
        "POSITION" => GeometryAttributeType::Position,
        "NORMAL" => GeometryAttributeType::Normal,
        "COLOR" => GeometryAttributeType::Color,
        "TEX_COORD" => GeometryAttributeType::TexCoord,
        "GENERIC" => GeometryAttributeType::Generic,
        _ => return None,
    })
}

fn semantic_name(attribute_type: GeometryAttributeType) -> &'static str {
    match attribute_type {
        GeometryAttributeType::Position => "POSITION",
        GeometryAttributeType::Normal => "NORMAL",
        GeometryAttributeType::Color => "COLOR",
        GeometryAttributeType::TexCoord => "TEX_COORD",
        _ => "GENERIC",
    }
}

fn target_from_name(name: &str) -> Option<Target> {
    Some(match name {
        "Int8Array" => Target::I8,
        "Uint8Array" => Target::U8,
        "Int16Array" => Target::I16,
        "Uint16Array" => Target::U16,
        "Int32Array" => Target::I32,
        "Uint32Array" => Target::U32,
        "Float32Array" => Target::F32,
        _ => return None,
    })
}

fn attribute_to_js(
    mesh: &Mesh,
    attribute: &PointAttribute,
    name: Option<&str>,
    array: JsValue,
) -> Object {
    let obj = Object::new();
    let name = name.map(str::to_string).or_else(|| {
        mesh.attribute_metadata_by_unique_id(attribute.unique_id())
            .and_then(|metadata| metadata.metadata().get_string("name").map(str::to_string))
    });
    set_opt_string(&obj, "name", &name);
    set_js(&obj, "uniqueId", &JsValue::from(attribute.unique_id()));
    set_js(
        &obj,
        "semantic",
        &JsValue::from_str(semantic_name(attribute.attribute_type())),
    );
    set_js(&obj, "array", &array);
    set_js(
        &obj,
        "itemSize",
        &JsValue::from(u32::from(attribute.num_components())),
    );
    set_bool(&obj, "normalized", attribute.normalized());
    obj
}

/// One stored component, as the C++ `ConvertComponentValue` sees its source.
#[derive(Clone, Copy)]
enum Source {
    Int {
        value: i128,
        signed: bool,
        bits: u32,
    },
    Bool(bool),
    Float {
        value: f64,
        single: bool,
    },
}

impl Target {
    fn is_float(self) -> bool {
        self == Target::F32
    }
    fn signed(self) -> bool {
        matches!(self, Target::I8 | Target::I16 | Target::I32)
    }
    fn bits(self) -> u32 {
        match self {
            Target::I8 | Target::U8 => 8,
            Target::I16 | Target::U16 => 16,
            Target::I32 | Target::U32 | Target::F32 => 32,
        }
    }
    fn min(self) -> i128 {
        if self.signed() {
            -(1i128 << (self.bits() - 1))
        } else {
            0
        }
    }
    fn max(self) -> i128 {
        if self.signed() {
            (1i128 << (self.bits() - 1)) - 1
        } else {
            (1i128 << self.bits()) - 1
        }
    }
}

/// An output component, held in the target's width.
#[derive(Clone, Copy)]
enum Value {
    Int(i128),
    Float(f32),
}

/// `ConvertComponentValue<T, OutT>` from Draco 1.5.7's `geometry_attribute.h`,
/// which `GetAttributeDataArrayForAllPoints` applies when the requested type
/// differs from the stored one. `None` where it returns false.
fn convert_component(source: Source, normalized: bool, target: Target) -> Option<Value> {
    if !target.is_float() {
        match source {
            Source::Int {
                value,
                signed,
                bits,
            } => {
                // C++ compares through the usual arithmetic conversions. A signed
                // source of `int`'s rank or narrower against a 32-bit unsigned
                // output is compared as `uint32_t`, so a negative value passes
                // and the cast below wraps it; every other pair compares as the
                // numbers they are.
                let wraps = signed && bits <= 32 && !target.signed() && target.bits() == 32;
                if !wraps {
                    let min = if signed { target.min() } else { 0 };
                    if value < min || value > target.max() {
                        return None;
                    }
                }
            }
            Source::Float { value, single } => {
                if value.is_nan() || value.is_infinite() {
                    return None;
                }
                // The integer limits convert to the source's floating type
                // before the comparison, which is what decides the edges.
                let (low, high) = if single {
                    (
                        f64::from(target.min() as f32),
                        f64::from(target.max() as f32),
                    )
                } else {
                    (target.min() as f64, target.max() as f64)
                };
                if value < low || value >= high {
                    return None;
                }
            }
            Source::Bool(_) => {}
        }
    }

    Some(match source {
        Source::Int {
            value,
            signed,
            bits,
        } if target.is_float() && normalized => {
            let source_max = if signed {
                (1i128 << (bits - 1)) - 1
            } else {
                (1i128 << bits) - 1
            };
            Value::Float(value as f32 / source_max as f32)
        }
        Source::Bool(value) if target.is_float() => Value::Float(f32::from(u8::from(value))),
        Source::Float { value, .. } if !target.is_float() && normalized => {
            if !(0.0..=1.0).contains(&value) {
                return None;
            }
            Value::Int((value * target.max() as f64 + 0.5).floor() as i128)
        }
        Source::Int { value, .. } => {
            if target.is_float() {
                Value::Float(value as f32)
            } else {
                Value::Int(value)
            }
        }
        Source::Bool(value) => Value::Int(i128::from(u8::from(value))),
        Source::Float { value, .. } => {
            if target.is_float() {
                // Exact from a float32 source, rounded to nearest from float64.
                Value::Float(value as f32)
            } else {
                // A cast to an integer truncates toward zero; the range check
                // above has kept it in range.
                Value::Int(value.trunc() as i128)
            }
        }
    })
}

fn read_component(data_type: DataType, bytes: &[u8]) -> Option<Source> {
    let int = |value: i128, signed: bool, bits: u32| {
        Some(Source::Int {
            value,
            signed,
            bits,
        })
    };
    match data_type {
        DataType::Int8 => int(
            i128::from(i8::from_le_bytes(bytes.try_into().ok()?)),
            true,
            8,
        ),
        DataType::Uint8 => int(i128::from(bytes[0]), false, 8),
        DataType::Int16 => int(
            i128::from(i16::from_le_bytes(bytes.try_into().ok()?)),
            true,
            16,
        ),
        DataType::Uint16 => int(
            i128::from(u16::from_le_bytes(bytes.try_into().ok()?)),
            false,
            16,
        ),
        DataType::Int32 => int(
            i128::from(i32::from_le_bytes(bytes.try_into().ok()?)),
            true,
            32,
        ),
        DataType::Uint32 => int(
            i128::from(u32::from_le_bytes(bytes.try_into().ok()?)),
            false,
            32,
        ),
        DataType::Int64 => int(
            i128::from(i64::from_le_bytes(bytes.try_into().ok()?)),
            true,
            64,
        ),
        DataType::Uint64 => int(
            i128::from(u64::from_le_bytes(bytes.try_into().ok()?)),
            false,
            64,
        ),
        DataType::Bool => Some(Source::Bool(bytes[0] != 0)),
        DataType::Float32 => Some(Source::Float {
            value: f64::from(f32::from_le_bytes(bytes.try_into().ok()?)),
            single: true,
        }),
        DataType::Float64 => Some(Source::Float {
            value: f64::from_le_bytes(bytes.try_into().ok()?),
            single: false,
        }),
        DataType::Invalid => None,
    }
}

/// Every point's components as the requested typed array, or in the stored
/// type when none is requested.
fn convert(
    mesh: &Mesh,
    attribute: &PointAttribute,
    target: Option<Target>,
) -> Result<JsValue, String> {
    let points = mesh.num_points();
    let components = usize::from(attribute.num_components());
    let data_type = attribute.data_type();

    // The common glTF case, and the one `read_f32s` is built for.
    if data_type == DataType::Float32 && matches!(target, None | Some(Target::F32)) {
        return Ok(Float32Array::from(attribute.read_f32s(points, components).as_slice()).into());
    }

    let width = data_type.byte_length();
    let stride = usize::try_from(attribute.byte_stride()).map_err(|_| "a negative byte stride")?;
    let data = attribute.buffer().data();
    let value_bytes = |point: usize, component: usize| -> Result<&[u8], String> {
        let index: AttributeValueIndex = attribute.mapped_index(PointIndex(point as u32));
        let start = (index.0 as usize)
            .checked_mul(stride)
            .and_then(|start| start.checked_add(component * width))
            .ok_or("an attribute value past the address space")?;
        data.get(start..start + width)
            .ok_or_else(|| format!("point {point} reads past the attribute's buffer"))
    };

    let Some(target) = target else {
        // The stored type, copied: nothing is converted, so nothing can fail.
        let mut bytes = Vec::with_capacity(points * components * width);
        for point in 0..points {
            for component in 0..components {
                bytes.extend_from_slice(value_bytes(point, component)?);
            }
        }
        return Ok(native_array(data_type, &bytes));
    };

    let normalized = attribute.normalized();
    let mut ints: Vec<i128> = Vec::new();
    let mut floats: Vec<f32> = Vec::new();
    for point in 0..points {
        for component in 0..components {
            let source = read_component(data_type, value_bytes(point, component)?)
                .ok_or("an attribute of an unsupported data type")?;
            match convert_component(source, normalized, target).ok_or_else(|| {
                format!(
                    "point {point} component {component} does not convert to the requested type"
                )
            })? {
                Value::Int(value) => ints.push(value),
                Value::Float(value) => floats.push(value),
            }
        }
    }
    // A cast to the narrower type wraps, as `static_cast` does; every value
    // here was range-checked except the ones C++ wraps too.
    Ok(match target {
        Target::F32 => Float32Array::from(floats.as_slice()).into(),
        Target::I8 => {
            Int8Array::from(ints.iter().map(|v| *v as i8).collect::<Vec<_>>().as_slice()).into()
        }
        Target::U8 => {
            Uint8Array::from(ints.iter().map(|v| *v as u8).collect::<Vec<_>>().as_slice()).into()
        }
        Target::I16 => Int16Array::from(
            ints.iter()
                .map(|v| *v as i16)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .into(),
        Target::U16 => Uint16Array::from(
            ints.iter()
                .map(|v| *v as u16)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .into(),
        Target::I32 => Int32Array::from(
            ints.iter()
                .map(|v| *v as i32)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .into(),
        Target::U32 => Uint32Array::from(
            ints.iter()
                .map(|v| *v as u32)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .into(),
    })
}

/// Little-endian bytes of the stored type, as the typed array of that type.
fn native_array(data_type: DataType, bytes: &[u8]) -> JsValue {
    fn values<const N: usize, T>(bytes: &[u8], from: fn([u8; N]) -> T) -> Vec<T> {
        bytes
            .as_chunks::<N>()
            .0
            .iter()
            .map(|chunk| from(*chunk))
            .collect()
    }
    match data_type {
        DataType::Int8 => Int8Array::from(values(bytes, i8::from_le_bytes).as_slice()).into(),
        DataType::Uint8 | DataType::Bool => Uint8Array::from(bytes).into(),
        DataType::Int16 => Int16Array::from(values(bytes, i16::from_le_bytes).as_slice()).into(),
        DataType::Uint16 => Uint16Array::from(values(bytes, u16::from_le_bytes).as_slice()).into(),
        DataType::Int32 => Int32Array::from(values(bytes, i32::from_le_bytes).as_slice()).into(),
        DataType::Uint32 => Uint32Array::from(values(bytes, u32::from_le_bytes).as_slice()).into(),
        DataType::Int64 => BigInt64Array::from(values(bytes, i64::from_le_bytes).as_slice()).into(),
        DataType::Uint64 => {
            BigUint64Array::from(values(bytes, u64::from_le_bytes).as_slice()).into()
        }
        DataType::Float32 => {
            Float32Array::from(values(bytes, f32::from_le_bytes).as_slice()).into()
        }
        DataType::Float64 => {
            Float64Array::from(values(bytes, f64::from_le_bytes).as_slice()).into()
        }
        DataType::Invalid => JsValue::NULL,
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_component, Source, Target, Value};

    fn int(value: Option<Value>) -> Option<i128> {
        match value? {
            Value::Int(value) => Some(value),
            Value::Float(_) => None,
        }
    }
    fn float(value: Option<Value>) -> Option<f32> {
        match value? {
            Value::Float(value) => Some(value),
            Value::Int(_) => None,
        }
    }
    fn i(value: i128, signed: bool, bits: u32) -> Source {
        Source::Int {
            value,
            signed,
            bits,
        }
    }

    /// The edges of `ConvertComponentValue`, each with what Draco 1.5.7
    /// answers for it.
    #[test]
    fn conversion_follows_draco_1_5_7() {
        // Integer to a narrower integer: range-checked.
        assert_eq!(
            int(convert_component(i(300, false, 16), false, Target::U8)),
            None
        );
        assert_eq!(
            int(convert_component(i(255, false, 16), false, Target::U8)),
            Some(255)
        );
        assert_eq!(
            int(convert_component(i(-1, true, 16), false, Target::U8)),
            None
        );
        assert_eq!(
            int(convert_component(i(-1, true, 16), false, Target::I8)),
            Some(-1)
        );
        // A signed source against uint32 compares as uint32 and wraps.
        assert_eq!(
            int(convert_component(i(-1, true, 8), false, Target::U32)),
            Some(-1)
        );
        assert_eq!(
            int(convert_component(i(-1, true, 32), false, Target::U32)),
            Some(-1)
        );
        // ...but a 64-bit one compares as itself.
        assert_eq!(
            int(convert_component(i(-1, true, 64), false, Target::U32)),
            None
        );
        // Float to integer: truncated, NaN and the top edge refused.
        let f = |value: f32| Source::Float {
            value: f64::from(value),
            single: true,
        };
        assert_eq!(int(convert_component(f(2.9), false, Target::I8)), Some(2));
        assert_eq!(int(convert_component(f(-2.9), false, Target::I8)), Some(-2));
        assert_eq!(int(convert_component(f(127.0), false, Target::I8)), None);
        assert_eq!(
            int(convert_component(f(f32::NAN), false, Target::I32)),
            None
        );
        // 2^31 as a float is the edge, and the largest float below it passes.
        assert_eq!(
            int(convert_component(f(2_147_483_648.0), false, Target::I32)),
            None
        );
        assert_eq!(
            int(convert_component(f(2_147_483_520.0), false, Target::I32)),
            Some(2_147_483_520)
        );
        // Normalized float to integer: [0, 1] only, rounded half up.
        assert_eq!(int(convert_component(f(0.5), true, Target::U8)), Some(128));
        assert_eq!(
            int(convert_component(f(1.0), true, Target::U16)),
            Some(65535)
        );
        assert_eq!(int(convert_component(f(1.5), true, Target::U8)), None);
        assert_eq!(int(convert_component(f(-0.1), true, Target::U8)), None);
        // Normalized integer to float: divided by the source type's maximum.
        assert_eq!(
            float(convert_component(i(255, false, 8), true, Target::F32)),
            Some(1.0)
        );
        assert_eq!(
            float(convert_component(i(-128, true, 8), true, Target::F32)),
            Some(-128.0 / 127.0)
        );
        // Not normalized: the value as it is.
        assert_eq!(
            float(convert_component(i(255, false, 8), false, Target::F32)),
            Some(255.0)
        );
        assert_eq!(
            int(convert_component(Source::Bool(true), false, Target::U16)),
            Some(1)
        );
    }
}
