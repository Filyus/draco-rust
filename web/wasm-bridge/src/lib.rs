//! The JavaScript conversion layer the WASM modules share.
//!
//! Values cross the boundary as plain JavaScript objects built and read by
//! hand rather than as serde structures: geometry goes over as typed arrays,
//! which cross in bulk and give JavaScript an array it owns outright, while
//! small fixed-size values and anything the shell checks with `Array.isArray`
//! cross as plain arrays. The cost of dropping serde is that nothing validates
//! untrusted input for us, so every field read from JavaScript is guarded here.
//!
//! These functions lived in five copies, one per module, and had already begun
//! to disagree: only `fbx-wasm` accepted a `Float64Array` where a `Float32Array`
//! was expected, and only `ply-wasm` and `drc-wasm` had learned to refuse float
//! colour arrays. A field that is valid in one module and rejected in the next
//! is a bug wherever the two meet, so the definitions live here once and every
//! module gets the union of what they had.
//!
//! Only the primitives belong here. What a mesh, a scene or an export result
//! looks like is each module's own business, and those composites stay beside
//! the writer that means them.

use js_sys::{
    Array, Float32Array, Float64Array, Int32Array, Object, Reflect, Uint16Array, Uint32Array,
    Uint8Array,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

// ===========================================================================
// Rust -> JavaScript
// ===========================================================================

/// Set a property on a JavaScript object.
///
/// A failed `Reflect::set` means the target rejected the write — a frozen
/// object or an exotic proxy, neither of which these modules ever build — so
/// there is nothing to report and nothing to recover.
pub fn set_js(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

pub fn set_bool(obj: &Object, key: &str, value: bool) {
    set_js(obj, key, &JsValue::from_bool(value));
}

pub fn set_u32(obj: &Object, key: &str, value: u32) {
    set_js(obj, key, &JsValue::from_f64(f64::from(value)));
}

pub fn set_i32(obj: &Object, key: &str, value: i32) {
    set_js(obj, key, &JsValue::from_f64(f64::from(value)));
}

pub fn set_f64(obj: &Object, key: &str, value: f64) {
    set_js(obj, key, &JsValue::from_f64(value));
}

/// Set an optional string, to `undefined` when absent, so the key stays present
/// with the same shape the serde output used to have.
pub fn set_opt_string(obj: &Object, key: &str, value: &Option<String>) {
    match value {
        Some(text) => set_js(obj, key, &JsValue::from_str(text)),
        None => set_js(obj, key, &JsValue::UNDEFINED),
    }
}

/// Set an optional string to `null` rather than `undefined` when absent, for
/// the fields whose readers distinguish the two.
pub fn set_opt_string_null(obj: &Object, key: &str, value: &Option<String>) {
    match value {
        Some(text) => set_js(obj, key, &JsValue::from_str(text)),
        None => set_js(obj, key, &JsValue::NULL),
    }
}

pub fn set_string_array(obj: &Object, key: &str, values: &[String]) {
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_str(value));
    }
    set_js(obj, key, &array.into());
}

// --- typed arrays ----------------------------------------------------------
//
// Each of these allocates a JavaScript array and copies into it, so what comes
// back belongs to JavaScript outright: nothing keeps a view into wasm memory,
// which would dangle the moment the heap grew, and nothing needs freeing.

pub fn f32_array_to_js(values: &[f32]) -> JsValue {
    Float32Array::from(values).into()
}

pub fn f64_array_to_js(values: &[f64]) -> JsValue {
    Float64Array::from(values).into()
}

pub fn u32_array_to_js(values: &[u32]) -> JsValue {
    Uint32Array::from(values).into()
}

pub fn i32_array_to_js(values: &[i32]) -> JsValue {
    Int32Array::from(values).into()
}

pub fn u16_array_to_js(values: &[u16]) -> JsValue {
    Uint16Array::from(values).into()
}

pub fn u8_array_to_js(values: &[u8]) -> JsValue {
    Uint8Array::from(values).into()
}

/// A small fixed-size value (a matrix row, a vec3 colour) that the shell checks
/// with `Array.isArray`, so it crosses as a plain array rather than a typed one.
///
/// The distinction is load-bearing rather than cosmetic: a typed array fails
/// `Array.isArray`, and the shell reads that as an absent field and drops it
/// without a word.
pub fn plain_f32_array_to_js(values: &[f32]) -> JsValue {
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_f64(f64::from(*value)));
    }
    array.into()
}

/// The same, for the integer fields the shell checks the same way — per-polygon
/// material assignments most of all.
pub fn plain_i32_array_to_js(values: &[i32]) -> JsValue {
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_f64(f64::from(*value)));
    }
    array.into()
}

// ===========================================================================
// JavaScript -> Rust
// ===========================================================================

/// Read a field that has to be present. An absent key reads back as `undefined`
/// from JavaScript, and `null` is how the shell spells a channel it deliberately
/// excluded; both mean the same thing here, which is that there is no value.
pub fn get_field(obj: &JsValue, key: &str) -> Option<JsValue> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) if value.is_undefined() || value.is_null() => None,
        Ok(value) => Some(value),
        Err(_) => None,
    }
}

pub fn opt_string_from_js(obj: &JsValue, key: &str) -> Option<String> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) if value.is_string() => value.as_string(),
        _ => None,
    }
}

pub fn opt_bool_from_js(obj: &JsValue, key: &str) -> Option<bool> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) => value.as_bool(),
        _ => None,
    }
}

pub fn opt_f64_from_js(obj: &JsValue, key: &str) -> Option<f64> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) => value.as_f64(),
        _ => None,
    }
}

pub fn opt_u32_from_js(obj: &JsValue, key: &str) -> Option<u32> {
    opt_f64_from_js(obj, key).map(|number| number as u32)
}

pub fn opt_i32_from_js(obj: &JsValue, key: &str) -> Option<i32> {
    opt_f64_from_js(obj, key).map(|number| number as i32)
}

/// One element of a plain array, checked against the range its slot admits.
///
/// The cast on its own is not enough. A float-to-integer `as` in Rust
/// saturates, so `-1` would land on vertex zero and `1e9` on the last vertex,
/// both as a silently wrong file rather than as a refusal.
pub fn whole_number(value: &JsValue, field: &str, min: f64, max: f64) -> Result<f64, String> {
    let number = value
        .as_f64()
        .ok_or_else(|| format!("{field} must contain only numbers"))?;
    if !number.is_finite() || number.fract() != 0.0 {
        return Err(format!("{field} must contain whole numbers"));
    }
    if number < min || number > max {
        return Err(format!("{field} must stay within {min}..={max}"));
    }
    Ok(number)
}

/// A float array from JavaScript: a typed array copies across in bulk, a plain
/// array is walked element by element. Both float widths are accepted, because
/// a JavaScript number is an `f64` and an array of them narrows either way.
pub fn f32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<f32>, String> {
    if let Some(typed) = value.dyn_ref::<Float32Array>() {
        return Ok(typed.to_vec());
    }
    if let Some(typed) = value.dyn_ref::<Float64Array>() {
        return Ok(typed
            .to_vec()
            .into_iter()
            .map(|value| value as f32)
            .collect());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Float32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as f32),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

pub fn f64_array_from_js(value: &JsValue, field: &str) -> Result<Vec<f64>, String> {
    if let Some(typed) = value.dyn_ref::<Float64Array>() {
        return Ok(typed.to_vec());
    }
    if let Some(typed) = value.dyn_ref::<Float32Array>() {
        return Ok(typed.to_vec().into_iter().map(f64::from).collect());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Float64Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

pub fn u32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<u32>, String> {
    if let Some(typed) = value.dyn_ref::<Uint32Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Uint32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        out.push(whole_number(&array.get(index), field, 0.0, f64::from(u32::MAX))? as u32);
    }
    Ok(out)
}

/// Signed, for the fields where the sign carries meaning: a negative entry in
/// FBX's `polygonVertexIndices` closes a polygon and `-1` in `materialIndices`
/// means no material. So this range admits negatives where the unsigned reader
/// does not, and stops at the `i32` ends rather than at zero.
pub fn i32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<i32>, String> {
    if let Some(typed) = value.dyn_ref::<Int32Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be an Int32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        out.push(whole_number(
            &array.get(index),
            field,
            f64::from(i32::MIN),
            f64::from(i32::MAX),
        )? as i32);
    }
    Ok(out)
}

/// A byte array from JavaScript.
///
/// Colours arrive already in the 0..255 byte domain: the shell scales every
/// float and normalized accessor before it calls in. A float typed array
/// therefore means the caller still holds 0..1 values, and casting those would
/// write an almost black mesh instead of failing — so the type is refused
/// rather than converted, because nothing here can tell 0..1 from 0..255 once
/// the values are in.
pub fn u8_array_from_js(value: &JsValue, field: &str) -> Result<Vec<u8>, String> {
    if let Some(typed) = value.dyn_ref::<Uint8Array>() {
        return Ok(typed.to_vec());
    }
    if value.dyn_ref::<Float32Array>().is_some() || value.dyn_ref::<Float64Array>().is_some() {
        return Err(format!(
            "{field} must hold 0..255 bytes; a float array is 0..1 data that the caller has to scale first"
        ));
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Uint8Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        out.push(whole_number(&array.get(index), field, 0.0, 255.0)? as u8);
    }
    Ok(out)
}

// --- required and optional fields ------------------------------------------
//
// The pairing every module needs: a field that must be there and reports its
// own name when it is not, and one that is allowed to be absent.

macro_rules! field_accessors {
    ($required:ident, $optional:ident, $reader:ident, $value:ty) => {
        pub fn $required(value: &JsValue, field: &str) -> Result<$value, String> {
            let value = get_field(value, field).ok_or_else(|| format!("{field} is required"))?;
            $reader(&value, field)
        }

        pub fn $optional(value: &JsValue, field: &str) -> Result<Option<$value>, String> {
            match get_field(value, field) {
                Some(value) => Ok(Some($reader(&value, field)?)),
                None => Ok(None),
            }
        }
    };
}

field_accessors!(
    required_f32_array,
    optional_f32_array,
    f32_array_from_js,
    Vec<f32>
);
field_accessors!(
    required_f64_array,
    optional_f64_array,
    f64_array_from_js,
    Vec<f64>
);
field_accessors!(
    required_u32_array,
    optional_u32_array,
    u32_array_from_js,
    Vec<u32>
);
field_accessors!(
    required_i32_array,
    optional_i32_array,
    i32_array_from_js,
    Vec<i32>
);
field_accessors!(
    required_u8_array,
    optional_u8_array,
    u8_array_from_js,
    Vec<u8>
);
