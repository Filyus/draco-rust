//! Atomic writing of materialized primitive geometry into lossless documents.

use crate::json::Value;
use crate::{
    ComponentType, Error, GeometryError, Import, MeshIndex, PackedAttribute, PackedGeometry,
    PrimitiveIndex, Result, ValidationProfile,
};

/// Storage selected for newly written primitive geometry.
#[derive(Clone, Copy, Debug, Default)]
pub enum GeometryEncoding {
    /// Store geometry in ordinary tightly packed glTF accessors.
    #[default]
    Raw,
    /// Encode geometry with `KHR_draco_mesh_compression`.
    #[cfg(feature = "draco-encode")]
    Draco(crate::CompressionOptions),
}

/// Options controlling one geometry write operation.
#[derive(Clone, Copy, Debug, Default)]
pub struct GeometryWriteOptions {
    /// Binary storage used by the destination primitive.
    pub encoding: GeometryEncoding,
}

/// Why source resource bytes were conservatively retained after a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreserveReason {
    /// Existing resources may still be referenced by another core object or
    /// by an extension whose binary-reference semantics are not remapped.
    ExistingReferences,
}

/// Measured result of one atomic geometry write.
#[derive(Clone, Debug)]
pub struct GeometryWriteReport {
    /// Primitive changed by the operation.
    pub primitive: PrimitiveIndex,
    /// Storage selected by the caller.
    pub encoding: GeometryEncoding,
    /// Total resolved bytes before the operation.
    pub source_bytes: usize,
    /// Total resolved bytes after the operation.
    pub output_bytes: usize,
    /// Bytes in the newly generated geometry payload.
    pub encoded_bytes: usize,
    /// Bytes proven unreachable and removed.
    pub reclaimed_bytes: usize,
    /// Reasons preventing more aggressive source-resource reclamation.
    pub preserve_reasons: Vec<PreserveReason>,
}

impl Import {
    /// Replaces one primitive's geometry atomically.
    ///
    /// Raw storage is the default. The existing material, `extras`, morph
    /// targets with the same vertex count, and unrelated extensions remain on
    /// the primitive. Shared source accessors are never modified in place.
    pub fn write_primitive(
        &mut self,
        primitive: PrimitiveIndex,
        geometry: &PackedGeometry,
        options: GeometryWriteOptions,
    ) -> Result<GeometryWriteReport> {
        geometry.validate(self.validation_profile())?;
        let mut candidate = self.clone();
        let source_bytes = total_bytes(&candidate)?;
        let raw_bytes = candidate.write_raw_primitive_inner(primitive, geometry)?;
        #[allow(unused_mut)]
        let mut report = GeometryWriteReport {
            primitive,
            encoding: options.encoding,
            source_bytes,
            output_bytes: total_bytes(&candidate)?,
            encoded_bytes: raw_bytes,
            reclaimed_bytes: 0,
            preserve_reasons: if source_bytes == 0 {
                Vec::new()
            } else {
                vec![PreserveReason::ExistingReferences]
            },
        };
        #[cfg(feature = "draco-encode")]
        if let GeometryEncoding::Draco(draco) = options.encoding {
            let compressed =
                candidate.compress_primitive(primitive.mesh, primitive.primitive, draco)?;
            report.output_bytes = compressed.output_bytes;
            report.encoded_bytes = compressed.encoded_bytes;
            report.reclaimed_bytes = compressed.reclaimed_bytes;
            if compressed.reclaimed_bytes > 0 {
                report.preserve_reasons.clear();
            }
        }
        candidate.validate_after_write()?;
        *self = candidate;
        Ok(report)
    }

    /// Appends packed geometry to an existing mesh atomically.
    ///
    /// Returns the stable location accepted by [`Import::read_primitive`] and
    /// [`Import::write_primitive`].
    pub fn push_primitive(
        &mut self,
        mesh: MeshIndex,
        geometry: &PackedGeometry,
        options: GeometryWriteOptions,
    ) -> Result<PrimitiveIndex> {
        geometry.validate(self.validation_profile())?;
        let mut candidate = self.clone();
        let primitives = candidate
            .document
            .as_value_mut()
            .get_mut("meshes")
            .and_then(Value::as_array_mut)
            .and_then(|meshes| meshes.get_mut(mesh.0))
            .and_then(|mesh| mesh.get_mut("primitives"))
            .and_then(Value::as_array_mut)
            .ok_or_else(|| Error::Validation(vec!["mesh primitives are invalid".into()]))?;
        let primitive = PrimitiveIndex::new(mesh, primitives.len());
        primitives.push(Value::object([("attributes", Value::Object(Vec::new()))]));
        candidate.write_primitive(primitive, geometry, options)?;
        *self = candidate;
        Ok(primitive)
    }

    /// Creates a minimal scene containing one packed primitive.
    ///
    /// ```
    /// use draco_gltf::{
    ///     ComponentType, GeometryWriteOptions, Import, PackedAttribute,
    ///     PackedGeometry, PrimitiveMode, ValidationProfile,
    /// };
    ///
    /// let position = PackedAttribute::new(
    ///     "POSITION", 1, 3, ComponentType::F32, false, vec![0; 12],
    /// )?;
    /// let geometry = PackedGeometry::new(PrimitiveMode::Points, vec![position], None)?;
    /// let scene = Import::from_geometry(
    ///     &geometry,
    ///     ValidationProfile::Gltf20,
    ///     GeometryWriteOptions::default(),
    /// )?;
    /// assert_eq!(scene.document.meshes().len(), 1);
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
    pub fn from_geometry(
        geometry: &PackedGeometry,
        profile: ValidationProfile,
        options: GeometryWriteOptions,
    ) -> Result<Self> {
        geometry.validate(profile)?;
        let version = match profile {
            ValidationProfile::Gltf20 => "2.0",
            ValidationProfile::Gltf21Draft => "2.1",
        };
        let document = format!(
            "{{\"asset\":{{\"version\":\"{version}\"}},\"buffers\":[],\"bufferViews\":[],\"accessors\":[],\"meshes\":[{{\"primitives\":[]}}],\"nodes\":[{{\"mesh\":0}}],\"scenes\":[{{\"nodes\":[0]}}],\"scene\":0}}"
        );
        let mut import = crate::parse(document.as_bytes(), profile)?;
        import.push_primitive(MeshIndex(0), geometry, options)?;
        Ok(import)
    }

    pub(crate) fn write_raw_primitive_inner(
        &mut self,
        location: PrimitiveIndex,
        geometry: &PackedGeometry,
    ) -> Result<usize> {
        let primitive = self
            .document
            .primitive(location.mesh, location.primitive)
            .ok_or_else(|| Error::Validation(vec!["primitive is out of range".into()]))?;
        validate_morph_targets(self, primitive, geometry.vertex_count())?;

        let buffer_index = self.resources.buffers.len();
        let mut bytes = Vec::new();
        let mut views = Vec::new();
        for attribute in geometry.attributes() {
            pad_to_four(&mut bytes);
            let offset = bytes.len();
            bytes.extend_from_slice(attribute.bytes());
            views.push((offset, attribute.bytes().len(), 34962u32));
        }
        let index_view = if let Some(indices) = geometry.indices() {
            pad_to_four(&mut bytes);
            let offset = bytes.len();
            bytes.extend_from_slice(indices.bytes());
            views.push((offset, indices.bytes().len(), 34963u32));
            Some(views.len() - 1)
        } else {
            None
        };
        let encoded_bytes = bytes.len();

        let root = self.document.as_value_mut();
        ensure_root_array(root, "buffers")?
            .push(Value::object([("byteLength", Value::from(bytes.len()))]));
        let first_view = ensure_root_array(root, "bufferViews")?.len();
        for (offset, length, target) in &views {
            ensure_root_array(root, "bufferViews")?.push(Value::object([
                ("buffer", Value::from(buffer_index)),
                ("byteOffset", Value::from(*offset)),
                ("byteLength", Value::from(*length)),
                ("target", Value::from(*target as u64)),
            ]));
        }

        let first_accessor = ensure_root_array(root, "accessors")?.len();
        for (offset, attribute) in geometry.attributes().iter().enumerate() {
            let mut accessor = Value::object([
                ("bufferView", Value::from(first_view + offset)),
                (
                    "componentType",
                    Value::from(attribute.component_type().to_gltf() as u64),
                ),
                ("count", Value::from(attribute.count())),
                ("type", Value::from(accessor_type(attribute.components()))),
            ]);
            if attribute.normalized() {
                accessor["normalized"] = Value::Bool(true);
            }
            if attribute.semantic() == "POSITION" {
                let (min, max) = position_bounds(attribute)?;
                accessor["min"] = Value::Array(min);
                accessor["max"] = Value::Array(max);
            }
            ensure_root_array(root, "accessors")?.push(accessor);
        }
        let index_accessor = if let (Some(indices), Some(view)) = (geometry.indices(), index_view) {
            let index = ensure_root_array(root, "accessors")?.len();
            ensure_root_array(root, "accessors")?.push(Value::object([
                ("bufferView", Value::from(first_view + view)),
                (
                    "componentType",
                    Value::from(indices.component_type().to_gltf() as u64),
                ),
                ("count", Value::from(indices.count())),
                ("type", Value::from("SCALAR")),
            ]));
            Some(index)
        } else {
            None
        };

        let primitive = root["meshes"][location.mesh.0]["primitives"]
            .as_array_mut()
            .and_then(|primitives| primitives.get_mut(location.primitive))
            .ok_or_else(|| Error::Validation(vec!["primitive changed during write".into()]))?;
        primitive["mode"] = Value::from(geometry.mode().to_gltf() as u64);
        primitive["attributes"] = Value::Object(
            geometry
                .attributes()
                .iter()
                .enumerate()
                .map(|(offset, attribute)| {
                    (
                        attribute.semantic().to_owned(),
                        Value::from(first_accessor + offset),
                    )
                })
                .collect(),
        );
        if let Some(index) = index_accessor {
            primitive["indices"] = Value::from(index);
        } else {
            remove_key(primitive, "indices");
        }
        remove_draco_extension(primitive);
        remove_unused_draco_name(root);
        self.resources.buffers.push(bytes);
        Ok(encoded_bytes)
    }
}

#[derive(Clone, Copy)]
enum BoundScalar {
    Signed(i64),
    Unsigned(u64),
    Float(f64),
}

impl BoundScalar {
    fn is_less_than(self, other: Self) -> bool {
        match (self, other) {
            (Self::Signed(left), Self::Signed(right)) => left < right,
            (Self::Unsigned(left), Self::Unsigned(right)) => left < right,
            (Self::Float(left), Self::Float(right)) => left < right,
            _ => unreachable!("one accessor cannot mix component types"),
        }
    }

    fn into_json(self) -> Value {
        let lexeme = match self {
            Self::Signed(value) => value.to_string(),
            Self::Unsigned(value) => value.to_string(),
            Self::Float(value) => finite_float_lexeme(value),
        };
        Value::Number(lexeme)
    }
}

pub(crate) fn finite_float_lexeme(value: f64) -> String {
    let bits = value.to_bits();
    let negative = bits >> 63 != 0;
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 && fraction == 0 {
        return if negative { "-0" } else { "0" }.into();
    }
    let (significand, binary_exponent) = if exponent == 0 {
        (fraction, -1074)
    } else {
        ((1u64 << 52) | fraction, exponent - 1023 - 52)
    };
    let mut digits = significand
        .to_string()
        .bytes()
        .rev()
        .map(|digit| digit - b'0')
        .collect::<Vec<_>>();
    let mut scale = 0usize;
    if binary_exponent >= 0 {
        for _ in 0..binary_exponent {
            multiply_decimal(&mut digits, 2);
        }
    } else {
        scale = (-binary_exponent) as usize;
        for _ in 0..scale {
            multiply_decimal(&mut digits, 5);
        }
        while scale > 0 && digits.first() == Some(&0) {
            digits.remove(0);
            scale -= 1;
        }
    }
    let mut out = String::with_capacity(digits.len() + 3 + scale.saturating_sub(digits.len()));
    if negative {
        out.push('-');
    }
    if scale == 0 {
        out.extend(digits.iter().rev().map(|digit| char::from(b'0' + digit)));
    } else if digits.len() > scale {
        for (index, digit) in digits.iter().rev().enumerate() {
            if index == digits.len() - scale {
                out.push('.');
            }
            out.push(char::from(b'0' + digit));
        }
    } else {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', scale - digits.len()));
        out.extend(digits.iter().rev().map(|digit| char::from(b'0' + digit)));
    }
    out
}

fn multiply_decimal(digits: &mut Vec<u8>, factor: u8) {
    let mut carry = 0u16;
    for digit in digits.iter_mut() {
        let value = u16::from(*digit) * u16::from(factor) + carry;
        *digit = (value % 10) as u8;
        carry = value / 10;
    }
    while carry != 0 {
        digits.push((carry % 10) as u8);
        carry /= 10;
    }
}

fn position_bounds(attribute: &PackedAttribute) -> Result<(Vec<Value>, Vec<Value>)> {
    let scalar_width = attribute.component_type().byte_width();
    let row_width = scalar_width
        .checked_mul(attribute.components() as usize)
        .ok_or(GeometryError::ByteSizeOverflow)?;
    let first = attribute
        .bytes()
        .get(..row_width)
        .ok_or(GeometryError::EmptyGeometry)?;
    let mut min = (0..attribute.components())
        .map(|component| {
            read_bound_scalar(
                first,
                component as usize * scalar_width,
                attribute.component_type(),
            )
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut max = min.clone();
    for row in attribute.bytes().chunks_exact(row_width).skip(1) {
        for component in 0..attribute.components() as usize {
            let value =
                read_bound_scalar(row, component * scalar_width, attribute.component_type())?;
            if value.is_less_than(min[component]) {
                min[component] = value;
            }
            if max[component].is_less_than(value) {
                max[component] = value;
            }
        }
    }
    Ok((
        min.into_iter().map(BoundScalar::into_json).collect(),
        max.into_iter().map(BoundScalar::into_json).collect(),
    ))
}

fn read_bound_scalar(
    bytes: &[u8],
    offset: usize,
    component_type: ComponentType,
) -> std::result::Result<BoundScalar, GeometryError> {
    let bytes = &bytes[offset..offset + component_type.byte_width()];
    let scalar = match component_type {
        ComponentType::I8 => BoundScalar::Signed(bytes[0] as i8 as i64),
        ComponentType::U8 => BoundScalar::Unsigned(bytes[0] as u64),
        ComponentType::I16 => {
            BoundScalar::Signed(i16::from_le_bytes(bytes.try_into().unwrap()) as i64)
        }
        ComponentType::U16 => {
            BoundScalar::Unsigned(u16::from_le_bytes(bytes.try_into().unwrap()) as u64)
        }
        ComponentType::I32 => {
            BoundScalar::Signed(i32::from_le_bytes(bytes.try_into().unwrap()) as i64)
        }
        ComponentType::U32 => {
            BoundScalar::Unsigned(u32::from_le_bytes(bytes.try_into().unwrap()) as u64)
        }
        ComponentType::F32 => {
            BoundScalar::Float(f32::from_le_bytes(bytes.try_into().unwrap()) as f64)
        }
        ComponentType::F16 => {
            BoundScalar::Float(half_to_f32(u16::from_le_bytes(bytes.try_into().unwrap())) as f64)
        }
        ComponentType::F64 => BoundScalar::Float(f64::from_le_bytes(bytes.try_into().unwrap())),
        ComponentType::I64 => BoundScalar::Signed(i64::from_le_bytes(bytes.try_into().unwrap())),
        ComponentType::U64 => BoundScalar::Unsigned(u64::from_le_bytes(bytes.try_into().unwrap())),
    };
    if matches!(scalar, BoundScalar::Float(value) if !value.is_finite()) {
        return Err(GeometryError::NonFinitePosition);
    }
    Ok(scalar)
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exponent = (bits >> 10) & 0x1f;
    let fraction = (bits & 0x03ff) as u32;
    let value = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let leading = fraction.leading_zeros() - 22;
            let normalized = fraction << (leading + 1);
            let exponent = 127 - 15 - leading;
            sign | (exponent << 23) | ((normalized & 0x03ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | ((exponent as u32 + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(value)
}

fn total_bytes(import: &Import) -> Result<usize> {
    import
        .resources
        .buffers
        .iter()
        .try_fold(0usize, |total, bytes| {
            total
                .checked_add(bytes.len())
                .ok_or_else(|| Error::ResourceLimit("total resource size overflow".into()))
        })
}

fn validate_morph_targets(
    import: &Import,
    primitive: crate::PrimitiveRef<'_>,
    vertex_count: usize,
) -> Result<()> {
    for target in primitive.morph_targets() {
        for (_, accessor) in target {
            let count = accessor
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| import.document.accessor(crate::AccessorIndex(index)))
                .and_then(|accessor| accessor.count())
                .and_then(|count| usize::try_from(count).ok())
                .ok_or_else(|| {
                    Error::Validation(vec!["morph target accessor is invalid".into()])
                })?;
            if count != vertex_count {
                return Err(Error::Geometry(crate::GeometryError::MorphTargetCount {
                    expected: count,
                    actual: vertex_count,
                }));
            }
        }
    }
    Ok(())
}

fn ensure_root_array<'a>(root: &'a mut Value, name: &str) -> Result<&'a mut Vec<Value>> {
    if root.get(name).is_none() {
        root[name] = Value::Array(Vec::new());
    }
    root.get_mut(name)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error::Validation(vec![format!("{name} is not an array")]))
}

fn accessor_type(components: u8) -> &'static str {
    match components {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        _ => unreachable!("PackedAttribute validates component counts"),
    }
}

fn pad_to_four(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn remove_key(value: &mut Value, key: &str) {
    if let Some(entries) = value.as_object_mut() {
        entries.retain(|(name, _)| name != key);
    }
}

fn remove_draco_extension(primitive: &mut Value) {
    let Some(extensions) = primitive
        .get_mut("extensions")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    extensions.retain(|(name, _)| name != crate::KHR_DRACO_MESH_COMPRESSION);
    if extensions.is_empty() {
        remove_key(primitive, "extensions");
    }
}

fn remove_unused_draco_name(root: &mut Value) {
    let still_used = root
        .get("meshes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|mesh| mesh.get("primitives").and_then(Value::as_array))
        .flatten()
        .any(|primitive| {
            primitive
                .get("extensions")
                .and_then(|extensions| extensions.get(crate::KHR_DRACO_MESH_COMPRESSION))
                .is_some()
        });
    if still_used {
        return;
    }
    for name in ["extensionsUsed", "extensionsRequired"] {
        if let Some(values) = root.get_mut(name).and_then(Value::as_array_mut) {
            values.retain(|value| value.as_str() != Some(crate::KHR_DRACO_MESH_COMPRESSION));
            if values.is_empty() {
                remove_key(root, name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::finite_float_lexeme;

    #[test]
    fn exact_float_lexemes_roundtrip() {
        for value in [
            0.0,
            -0.0,
            0.1,
            -12345.75,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::MAX,
        ] {
            let parsed = finite_float_lexeme(value).parse::<f64>().unwrap();
            assert_eq!(parsed.to_bits(), value.to_bits());
        }
    }
}
