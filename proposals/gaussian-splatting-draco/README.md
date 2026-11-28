# TEST_gaussian_splatting_compression_draco

**Status** · Private working draft with a provisional identifier.

## Motivation

A mixed glTF scene can contain both triangle meshes and Gaussian splats.
[KHR_draco_mesh_compression](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_draco_mesh_compression)
covers triangle primitives. [KHR_gaussian_splatting](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_gaussian_splatting)
uses `POINTS`.

This extension lets a Gaussian splat primitive reference one Draco point-cloud
stream. glTF still defines the attributes and node transforms. A single
Draco decoder with support for both geometry types can serve a scene that
uses Draco for both meshes and splats.

## Dependencies

- This extension depends on [glTF 2.0](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)
  and `KHR_gaussian_splatting`.
- The payload MUST use Draco point-cloud bitstream version **2.3**.
- Uppercase requirement keywords follow [BCP 14](https://www.rfc-editor.org/rfc/rfc8174).

## Extension placement and declarations

| Location | Requirement |
| --- | --- |
| Mesh primitive | The primitive MUST specify `mode: 0` (`POINTS`) and omit `indices`. |
| `primitive.extensions.KHR_gaussian_splatting.extensions` | The child extension object MUST appear here. |
| Asset root | Both extension identifiers MUST appear in both `extensionsUsed` and `extensionsRequired`. |

The Draco payload supplies every attribute of the primitive. Another extension
MUST NOT supply the same attribute data.

### Example (informative)

This primitive uses degree-zero lighting. Its `POSITION` is accessor `5` in
the glTF document and attribute ID `0` in the Draco stream.

```json
{
  "mode": 0,
  "attributes": {
    "POSITION": 5,
    "KHR_gaussian_splatting:ROTATION": 6,
    "KHR_gaussian_splatting:SCALE": 7,
    "KHR_gaussian_splatting:OPACITY": 8,
    "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0": 9
  },
  "extensions": {
    "KHR_gaussian_splatting": {
      "kernel": "ellipse",
      "colorSpace": "srgb_rec709_display",
      "extensions": {
        "TEST_gaussian_splatting_compression_draco": {
          "bufferView": 1,
          "attributes": {
            "POSITION": 0,
            "KHR_gaussian_splatting:ROTATION": 1,
            "KHR_gaussian_splatting:SCALE": 2,
            "KHR_gaussian_splatting:OPACITY": 3,
            "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0": 4
          }
        }
      }
    }
  }
}
```

## Extension properties

| Property | Type | Required | Meaning |
| --- | --- | --- | --- |
| `bufferView` | integer ≥ 0 | Yes | Index of the buffer view containing one complete Draco point-cloud stream. |
| `attributes` | object | Yes | Map from primitive attribute semantics to unsigned Draco unique attribute IDs. |

The payload buffer view holds opaque bytes and MUST omit `byteStride`.

The keys of the extension's `attributes` MUST match those of
`primitive.attributes`, including application-specific semantics. Each ID
MUST resolve to a distinct attribute in the stream. Readers MUST look up
each attribute by its Draco unique ID.

## Decoded accessors

### Layout and interpretation

| Accessor field | Requirement |
| --- | --- |
| `bufferView`, `byteOffset`, `sparse` | Each mapped accessor MUST omit these fields. |
| `count` | MUST equal the decoded point count. |
| `type`, `componentType` | MUST describe the component count and scalar representation after decoding and reversing Draco attribute transforms. |
| `normalized` | The reader MUST apply the accessor's integer normalization exactly once. |
| `POSITION` bounds | MUST describe the decoded positions. |

For each base splat semantic, the combination of `type`, `componentType` and
`normalized` MUST be valid under `KHR_gaussian_splatting`. Each SH
coefficient uses a Float32 `VEC3`.

All attributes MUST be interpreted independently of Draco metadata.

### Point order and multiplicity

- Readers MUST obtain each point's values through the respective Draco
  point-to-attribute mappings.
- Writers MAY reorder whole points. They MUST preserve multiplicity and
  MUST NOT merge or discard points, even if attributes coincide before or
  after quantization.
- Applications MAY store stable identifiers in a custom attribute.

### Decoded values

Loaders MUST reject NaN and either infinity in any decoded floating-point
attribute, including application-specific attributes. See
[glTF accessor data types](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#accessor-data-types).

Splat values follow the base extension's semantics and ranges, with the
following rules.

| Attribute | Requirement |
| --- | --- |
| `SCALE` and `OPACITY` | Loaders MUST reject values strictly less than zero. Both signs of zero are valid. |
| Float32 `OPACITY` | Loaders MUST clamp the immediate successor of 1 (1 + 2⁻²³) to 1 and MUST reject larger values. |
| `ROTATION` | Writers MUST encode unit quaternions to the precision of the selected representation. Each decoded quaternion MUST be finite and nonzero. |

## Loader conformance

The glTF rules for unsupported required extensions apply to the whole asset.

For each primitive using this extension, a conforming loader MUST perform
the following steps.

1. Resolve the buffer view and decode its byte range as one Draco 2.3 point
   cloud. The loader MUST support both sequential and KD-tree encoding.
2. Check the attribute mappings and decoded accessor contract. Reject an
   invalid payload, a conflicting attribute source, an unresolved reference,
   or a contract mismatch.
3. Expose the resulting attribute values to the base splat extension in
   decoded point order.

## Renderer conformance

### Quaternion normalization

After accessor normalization and before using `ROTATION`, a renderer MUST
divide the quaternion by its Euclidean norm. The computation MUST avoid
intermediate overflow and underflow to zero.

### Directional spherical harmonics

When evaluating directional SH, a renderer MUST account for the rotation
of the global transform of **each instance** of the splat primitive.

An instance is each use of the primitive under its own global transform.
This includes instances of a node defined by
[EXT_mesh_gpu_instancing](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Vendor/EXT_mesh_gpu_instancing).
For those instances, the renderer uses the rotation of the node's global
transform multiplied on the right by the per-instance transform.

Given the world-space camera-to-splat unit direction `d` and the instance
rotation `R`, a renderer can evaluate local SH at `transpose(R) * d` or
rotate the coefficients into world space. The base extension defines the
supported node transforms and SH conventions.

## Encoder guidance (informative)

- **Include zero on the grid when practical.** A grid that reconstructs zero
  exactly preserves zero SH coefficients and zero components of `ROTATION`.
  For example, a zero scalar component preserves a quaternion's half-turn.
  The grid needs to cover the source range at the required precision.
- **Use a common step within an SH degree when appropriate.** This gives its
  coefficients the same error budget. Rotation in the orthonormal SH basis
  preserves the L2 norm of an existing error vector within a complete degree.
  A second quantization can introduce new error even with a common step.
- **Use a node rotation for changes of axes.** This keeps the original SH
  coefficients and avoids a second quantization.
- **Choose precision and point order for the asset.** The decoded accessors
  and point multiplicity still need to satisfy the contract.

## Implementation notes (informative)

### Applying integer normalization once

Some Draco APIs apply attribute normalization when returning floating-point
values. An implementation can read raw integer components and apply the
accessor flag itself. This conversion is separate from dividing a decoded
quaternion by its norm.

### Computing the quaternion norm safely

Computing the norm and division in Float64 suffices for Float32 input:

- Each nonzero finite Float32 component has a square in `[2⁻²⁹⁸, 2²⁵⁶)`.
- The sum of four squares is less than `2²⁵⁸`.

These magnitudes lie within the normal Float64 range.

### Bounding quaternion quantization error

For round-to-nearest SNORM with positive endpoint integer `M`, a unit
quaternion has a component error of at most `1/(2M)`. Its four-component
error norm, and therefore the deviation of its norm from 1, is at most
`1/M`. At least one input component has magnitude at least `1/2`, so
direct SNORM rounding of a unit input cannot produce the zero quaternion.

For Draco's uniform Float32 quaternion quantizer, let `r` be the common
range and `N = 2^b − 1` the number of intervals. In exact arithmetic:

- Each component has error at most `r/(2N)`.
- The quaternion's norm deviates from 1 by at most `r/N`.
- If `N > r`, at least one decoded component stays nonzero.

Implementation error bounds also need to account for Float32 rounding.
The finite, nonzero check still applies at decode time.

### Why the opacity limit is one Float32 step above 1

Consider a grid with endpoints in [0, 1] and exactly representable integer
codes `0 ≤ q ≤ N`. Evaluate `min + q · (range/N)` in Float32, rounding to
nearest at each step. Let `u = 2⁻²⁴`.

| Source of error | Maximum contribution near the upper endpoint |
| --- | --- |
| Subtraction to derive the range | `u/2` |
| Rounding the step | `u` |
| Rounding the product | `u` |

Before final rounding, the result can exceed 1 by at most `2.5u`. This is
below `1 + 3u`, the midpoint at which rounding would produce the second
Float32 value above 1. A nonnegative minimum, step and code also keep the
result nonnegative.

The [Draco sequential dequantization pseudocode](https://google.github.io/draco/spec/#sequential-quantization-attribute-decoder)
evaluates `q/N` first using a rounded reciprocal. The reciprocal contributes
at most `u` after scaling. Each product rounds by at most `u/2` in [0, 1].
This gives the same arithmetic error bound of `2u` before the final addition.
Evaluations with no greater rounding error have the same endpoint bound.

## JSON Schema

The [extension object schema](schema/gaussian-splatting-compression-draco.schema.json)
covers the JSON object. The preceding requirements also govern cross-references,
decoded values and the enclosing glTF objects.

## Samples and related work

- [Interoperability sample](sample.md) and [conformance vectors](vectors/README.md)
- [SPZ child extension draft](https://github.com/KhronosGroup/glTF/pull/2531)
- [KHR_meshopt_compression](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_meshopt_compression)
