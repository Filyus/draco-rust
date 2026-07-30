//! Internal C++ test bridge for Rust parity and performance tests
//!
//! This crate is not a public C API surface. It provides a private bridge to
//! the original C++ Draco encoder/decoder for Rust parity and performance tests.

#[cfg(not(cpp_test_bridge_disabled))]
mod ffi {
    use std::os::raw::c_int;

    /// Profiling result structure from C++
    #[repr(C)]
    pub struct DracoProfileResult {
        pub mesh_setup_us: i64,
        pub encoder_setup_us: i64,
        pub encode_time_us: i64,
        pub total_time_us: i64,
        pub output_size: usize,
    }

    /// Re-encode profiling result structure from C++.
    #[repr(C)]
    pub struct DracoReencodeProfileResult {
        pub decode_setup_us: i64,
        pub encoder_setup_us: i64,
        pub encode_time_us: i64,
        pub total_encode_us: i64,
        pub output_size: usize,
        pub num_points: u32,
        pub num_faces: u32,
        pub num_attributes: u32,
    }

    /// Decode profiling result structure from C++
    #[repr(C)]
    pub struct DracoDecodeProfileResult {
        pub decode_time_us: i64,
        pub num_points: u32,
        pub num_faces: u32,
    }

    /// Decoded mesh fingerprint from C++.
    #[repr(C)]
    pub struct DracoDecodeFingerprint {
        pub num_points: u32,
        pub num_faces: u32,
        pub num_attributes: u32,
        pub face_hash: u64,
        pub attribute_hash: u64,
        pub canonical_corner_hash: u64,
    }

    extern "C" {
        /// Benchmark encoding: runs encoding multiple times and returns average time in microseconds
        pub fn draco_benchmark_encode_mesh(
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
            iterations: u32,
            output_size: *mut usize,
        ) -> i64;

        /// Single-shot encoding that returns encoded bytes
        pub fn draco_encode_mesh_single(
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
            output_buffer: *mut u8,
            output_buffer_size: usize,
        ) -> usize;

        #[allow(clippy::too_many_arguments)]
        pub fn draco_encode_mesh_attributed(
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            normals: *const f32,
            uvs: *const f32,
            colors: *const u8,
            encoding_speed: c_int,
            decoding_speed: c_int,
            position_bits: c_int,
            normal_bits: c_int,
            uv_bits: c_int,
            color_bits: c_int,
            output_buffer: *mut u8,
            output_buffer_size: usize,
        ) -> usize;

        /// Mesh whose attributes carry explicit point maps, so one vertex can
        /// hold several UVs and the encoder has to emit attribute seams.
        #[allow(clippy::too_many_arguments)]
        pub fn draco_encode_mesh_seamed(
            num_points: u32,
            num_position_values: u32,
            positions: *const f32,
            position_map: *const u32,
            num_uv_values: u32,
            uvs: *const f32,
            uv_map: *const u32,
            num_faces: u32,
            faces: *const u32,
            encoding_speed: c_int,
            decoding_speed: c_int,
            position_bits: c_int,
            uv_bits: c_int,
            output_buffer: *mut u8,
            output_buffer_size: usize,
        ) -> usize;
        /// Single-shot sequential mesh encoding with optional compressed connectivity.
        pub fn draco_encode_mesh_sequential(
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
            compress_connectivity: c_int,
            output_buffer: *mut u8,
            output_buffer_size: usize,
        ) -> usize;

        /// Create/free mesh handles
        pub fn draco_create_mesh() -> *mut ::std::ffi::c_void;
        pub fn draco_free_mesh(handle: *mut ::std::ffi::c_void);

        /// Mesh setup helpers
        pub fn draco_mesh_set_num_faces(handle: *mut ::std::ffi::c_void, num_faces: u32);
        pub fn draco_mesh_set_face(
            handle: *mut ::std::ffi::c_void,
            face_idx: u32,
            v0: u32,
            v1: u32,
            v2: u32,
        );
        pub fn draco_mesh_add_position_attribute(
            handle: *mut ::std::ffi::c_void,
            num_points: u32,
            positions: *const f32,
        ) -> c_int;

        /// Encoder buffer helpers
        pub fn draco_create_encoder_buffer() -> *mut ::std::ffi::c_void;
        pub fn draco_free_encoder_buffer(handle: *mut ::std::ffi::c_void);
        pub fn draco_encoder_buffer_data(handle: *mut ::std::ffi::c_void) -> *const u8;
        pub fn draco_encoder_buffer_size(handle: *mut ::std::ffi::c_void) -> usize;

        /// Encode using handles (mesh -> encoder buffer)
        pub fn draco_encode_mesh(
            mesh_handle: *mut ::std::ffi::c_void,
            buffer_handle: *mut ::std::ffi::c_void,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
        ) -> i64;

        /// Get version info for verification
        pub fn draco_get_version(major: *mut c_int, minor: *mut c_int, revision: *mut c_int);

        /// Detailed profiling of encoding stages
        pub fn draco_profile_encode(
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
            iterations: u32,
            result: *mut DracoProfileResult,
        ) -> c_int;

        /// Profile re-encoding a mesh decoded from C++ .drc bytes.
        pub fn draco_profile_reencode_mesh(
            encoded_data: *const u8,
            encoded_size: usize,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
            iterations: u32,
            result: *mut DracoReencodeProfileResult,
        ) -> c_int;

        /// Benchmark decoding
        pub fn draco_benchmark_decode_mesh(
            encoded_data: *const u8,
            encoded_size: usize,
            iterations: u32,
            out_num_points: *mut u32,
            out_num_faces: *mut u32,
        ) -> i64;

        /// Profile decoding with detailed timing
        pub fn draco_profile_decode(
            encoded_data: *const u8,
            encoded_size: usize,
            iterations: u32,
            result: *mut DracoDecodeProfileResult,
        ) -> c_int;

        /// Decode a mesh once and return stable structural/data fingerprints.
        pub fn draco_decode_attribute_values(
            encoded_data: *const u8,
            encoded_size: usize,
            attribute_type: c_int,
            output: *mut f32,
            output_capacity: usize,
        ) -> usize;

        pub fn draco_decode_mesh_fingerprint(
            encoded_data: *const u8,
            encoded_size: usize,
            result: *mut DracoDecodeFingerprint,
        ) -> c_int;

        /// Decode a point cloud once and return stable structural/data fingerprints.
        pub fn draco_decode_point_cloud_fingerprint(
            encoded_data: *const u8,
            encoded_size: usize,
            result: *mut DracoDecodeFingerprint,
        ) -> c_int;
    }
}

#[cfg(not(cpp_test_bridge_disabled))]
pub use ffi::*;

/// Detailed profiling result from C++ encoder
#[derive(Debug, Clone)]
pub struct CppProfileResult {
    pub mesh_setup_us: i64,
    pub encoder_setup_us: i64,
    pub encode_time_us: i64,
    pub total_time_us: i64,
    pub output_size: usize,
}

/// Detailed profiling result from re-encoding a C++-decoded mesh.
#[derive(Debug, Clone)]
pub struct CppReencodeProfileResult {
    pub decode_setup_us: i64,
    pub encoder_setup_us: i64,
    pub encode_time_us: i64,
    pub total_encode_us: i64,
    pub output_size: usize,
    pub num_points: u32,
    pub num_faces: u32,
    pub num_attributes: u32,
}

/// Check if the C++ test bridge is available
pub fn is_available() -> bool {
    #[cfg(cpp_test_bridge_disabled)]
    return false;

    #[cfg(not(cpp_test_bridge_disabled))]
    return true;
}

/// Get Draco C++ library version
#[cfg(not(cpp_test_bridge_disabled))]
pub fn get_version() -> (i32, i32, i32) {
    let mut major = 0;
    let mut minor = 0;
    let mut revision = 0;
    unsafe {
        draco_get_version(&mut major, &mut minor, &mut revision);
    }
    (major, minor, revision)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn get_version() -> (i32, i32, i32) {
    (0, 0, 0)
}

/// Benchmark result from C++ encoder
pub struct CppBenchmarkResult {
    pub avg_time_us: i64,
    pub output_size: usize,
}

/// Benchmark the C++ encoder with given mesh data
///
/// # Arguments
/// * `positions` - Flat array of f32 positions (num_points * 3 values)
/// * `faces` - Flat array of face indices (num_faces * 3 values)
/// * `encoding_speed` - Encoding speed (0 = best compression, 10 = fastest)
/// * `decoding_speed` - Decoding speed (0 = best compression, 10 = fastest)
/// * `quantization_bits` - Quantization bits for position attribute
/// * `iterations` - Number of iterations to average
///
/// # Returns
/// * `Some(CppBenchmarkResult)` if the C++ test bridge is available and encoding succeeded
/// * `None` if the C++ test bridge is disabled or encoding failed
#[cfg(not(cpp_test_bridge_disabled))]
pub fn benchmark_cpp_encode(
    positions: &[f32],
    faces: &[u32],
    encoding_speed: i32,
    decoding_speed: i32,
    quantization_bits: i32,
    iterations: u32,
) -> Option<CppBenchmarkResult> {
    let num_points = (positions.len() / 3) as u32;
    let num_faces = (faces.len() / 3) as u32;

    let mut output_size: usize = 0;

    let avg_time_us = unsafe {
        draco_benchmark_encode_mesh(
            num_points,
            positions.as_ptr(),
            num_faces,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
            iterations,
            &mut output_size,
        )
    };

    if avg_time_us < 0 {
        return None;
    }

    Some(CppBenchmarkResult {
        avg_time_us,
        output_size,
    })
}

#[cfg(cpp_test_bridge_disabled)]
pub fn benchmark_cpp_encode(
    _positions: &[f32],
    _faces: &[u32],
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
    _iterations: u32,
) -> Option<CppBenchmarkResult> {
    None
}

/// Stub for builds without the C++ Draco library, returning `-1`.
///
/// # Safety
///
/// This body dereferences nothing, but the signature mirrors the `extern "C"`
/// declaration it stands in for, so callers must still uphold that contract:
/// the pointers must be valid for the given counts. The argument count is the
/// C function's, not a choice made here.
#[cfg(cpp_test_bridge_disabled)]
#[allow(clippy::too_many_arguments)]
pub unsafe fn draco_benchmark_encode_mesh(
    _num_points: u32,
    _positions: *const f32,
    _num_faces: u32,
    _faces: *const u32,
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
    _iterations: u32,
    _output_size: *mut usize,
) -> i64 {
    -1
}

/// Encode a mesh using C++ Draco and return the encoded bytes
#[cfg(not(cpp_test_bridge_disabled))]
pub fn encode_cpp_mesh(
    positions: &[f32],
    faces: &[u32],
    encoding_speed: i32,
    decoding_speed: i32,
    quantization_bits: i32,
) -> Option<Vec<u8>> {
    let num_points = (positions.len() / 3) as u32;
    let num_faces = (faces.len() / 3) as u32;

    // Allocate enough space for both compressed and fast sequential outputs.
    // Fast sequential connectivity can be much larger than the compressed
    // low-speed streams, especially on dense grids.
    let buffer_size = (num_points as usize * 12 + faces.len() * 4 + 4096).max(65536);
    let mut buffer = vec![0u8; buffer_size];

    let encoded_size = unsafe {
        ffi::draco_encode_mesh_single(
            num_points,
            positions.as_ptr(),
            num_faces,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
            buffer.as_mut_ptr(),
            buffer_size,
        )
    };

    if encoded_size == 0 {
        return None;
    }

    buffer.truncate(encoded_size);
    Some(buffer)
}

/// Draco's own attribute type numbering, for the decode helper below.
pub mod cpp_attribute {
    /// `GeometryAttribute::POSITION`.
    pub const POSITION: i32 = 0;
    /// `GeometryAttribute::NORMAL`.
    pub const NORMAL: i32 = 1;
    /// `GeometryAttribute::COLOR`.
    pub const COLOR: i32 = 2;
    /// `GeometryAttribute::TEX_COORD`.
    pub const TEX_COORD: i32 = 3;
}

/// Decode a payload with C++ Draco and return one attribute's values.
///
/// The fingerprint helpers answer "same or not". This answers "how far apart",
/// which is what separates an encoder defect from a decoder one: read the same
/// payload with both implementations and see which pair agrees.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn decode_cpp_attribute_values(encoded: &[u8], attribute_type: i32) -> Option<Vec<f32>> {
    // Generous: four components per point at the largest point count these
    // tests build, and the call reports the true length back.
    let mut values = vec![0.0f32; 1 << 20];
    let written = unsafe {
        ffi::draco_decode_attribute_values(
            encoded.as_ptr(),
            encoded.len(),
            attribute_type,
            values.as_mut_ptr(),
            values.len(),
        )
    };
    if written == 0 {
        return None;
    }
    values.truncate(written);
    Some(values)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn decode_cpp_attribute_values(_encoded: &[u8], _attribute_type: i32) -> Option<Vec<f32>> {
    None
}

/// What a mesh carries besides positions. `None` means the attribute is absent.
///
/// Positions alone cannot show whether the two encoders agree: the prediction
/// schemes that could differ belong to normals and texture coordinates.
#[derive(Debug, Default, Clone, Copy)]
pub struct CppMeshAttributes<'a> {
    /// Three floats per point.
    pub normals: Option<&'a [f32]>,
    /// Two floats per point.
    pub uvs: Option<&'a [f32]>,
    /// Four normalized bytes per point.
    pub colors: Option<&'a [u8]>,
    /// Quantization bits, one per attribute, in the same order.
    pub normal_bits: i32,
    pub uv_bits: i32,
    pub color_bits: i32,
}

/// Encode a mesh with attributes through C++ Draco and return the bytes.
///
/// Attributes are added in the order normal, texture coordinate, colour. Draco
/// numbers attributes by insertion and encodes them in that order, so a caller
/// comparing against another encoder has to add them the same way.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn encode_cpp_mesh_attributed(
    positions: &[f32],
    faces: &[u32],
    attributes: CppMeshAttributes<'_>,
    encoding_speed: i32,
    decoding_speed: i32,
    position_bits: i32,
) -> Option<Vec<u8>> {
    let num_points = (positions.len() / 3) as u32;
    let num_faces = (faces.len() / 3) as u32;

    let buffer_size = (num_points as usize * 32 + faces.len() * 4 + 4096).max(65536);
    let mut buffer = vec![0u8; buffer_size];

    let encoded_size = unsafe {
        ffi::draco_encode_mesh_attributed(
            num_points,
            positions.as_ptr(),
            num_faces,
            faces.as_ptr(),
            attributes
                .normals
                .map_or(std::ptr::null(), |values| values.as_ptr()),
            attributes
                .uvs
                .map_or(std::ptr::null(), |values| values.as_ptr()),
            attributes
                .colors
                .map_or(std::ptr::null(), |values| values.as_ptr()),
            encoding_speed,
            decoding_speed,
            position_bits,
            attributes.normal_bits,
            attributes.uv_bits,
            attributes.color_bits,
            buffer.as_mut_ptr(),
            buffer_size,
        )
    };

    if encoded_size == 0 {
        return None;
    }

    buffer.truncate(encoded_size);
    Some(buffer)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn encode_cpp_mesh_attributed(
    _positions: &[f32],
    _faces: &[u32],
    _attributes: CppMeshAttributes<'_>,
    _encoding_speed: i32,
    _decoding_speed: i32,
    _position_bits: i32,
) -> Option<Vec<u8>> {
    None
}

#[cfg(cpp_test_bridge_disabled)]
pub fn encode_cpp_mesh(
    _positions: &[f32],
    _faces: &[u32],
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
) -> Option<Vec<u8>> {
    None
}

/// Encode a mesh using C++ Draco sequential mode.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn encode_cpp_mesh_sequential(
    positions: &[f32],
    faces: &[u32],
    encoding_speed: i32,
    decoding_speed: i32,
    quantization_bits: i32,
    compress_connectivity: bool,
) -> Option<Vec<u8>> {
    let num_points = (positions.len() / 3) as u32;
    let num_faces = (faces.len() / 3) as u32;

    let buffer_size = (num_points as usize * 12 + faces.len() * 4 + 4096).max(65536);
    let mut buffer = vec![0u8; buffer_size];

    let encoded_size = unsafe {
        ffi::draco_encode_mesh_sequential(
            num_points,
            positions.as_ptr(),
            num_faces,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
            i32::from(compress_connectivity),
            buffer.as_mut_ptr(),
            buffer_size,
        )
    };

    if encoded_size == 0 {
        return None;
    }

    buffer.truncate(encoded_size);
    Some(buffer)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn encode_cpp_mesh_sequential(
    _positions: &[f32],
    _faces: &[u32],
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
    _compress_connectivity: bool,
) -> Option<Vec<u8>> {
    None
}

/// Profile C++ encoding with detailed timing breakdown
#[cfg(not(cpp_test_bridge_disabled))]
pub fn profile_cpp_encode(
    positions: &[f32],
    faces: &[u32],
    encoding_speed: i32,
    decoding_speed: i32,
    quantization_bits: i32,
    iterations: u32,
) -> Option<CppProfileResult> {
    let num_points = (positions.len() / 3) as u32;
    let num_faces = (faces.len() / 3) as u32;

    let mut result = ffi::DracoProfileResult {
        mesh_setup_us: 0,
        encoder_setup_us: 0,
        encode_time_us: 0,
        total_time_us: 0,
        output_size: 0,
    };

    let status = unsafe {
        ffi::draco_profile_encode(
            num_points,
            positions.as_ptr(),
            num_faces,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
            iterations,
            &mut result,
        )
    };

    if status != 0 {
        return None;
    }

    Some(CppProfileResult {
        mesh_setup_us: result.mesh_setup_us,
        encoder_setup_us: result.encoder_setup_us,
        encode_time_us: result.encode_time_us,
        total_time_us: result.total_time_us,
        output_size: result.output_size,
    })
}

#[cfg(cpp_test_bridge_disabled)]
pub fn profile_cpp_encode(
    _positions: &[f32],
    _faces: &[u32],
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
    _iterations: u32,
) -> Option<CppProfileResult> {
    None
}

/// Profile C++ re-encoding of a mesh decoded from existing .drc bytes.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn profile_cpp_reencode_mesh(
    encoded_data: &[u8],
    encoding_speed: i32,
    decoding_speed: i32,
    quantization_bits: i32,
    iterations: u32,
) -> Option<CppReencodeProfileResult> {
    let mut result = ffi::DracoReencodeProfileResult {
        decode_setup_us: 0,
        encoder_setup_us: 0,
        encode_time_us: 0,
        total_encode_us: 0,
        output_size: 0,
        num_points: 0,
        num_faces: 0,
        num_attributes: 0,
    };

    let status = unsafe {
        ffi::draco_profile_reencode_mesh(
            encoded_data.as_ptr(),
            encoded_data.len(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
            iterations,
            &mut result,
        )
    };

    if status != 0 {
        return None;
    }

    Some(CppReencodeProfileResult {
        decode_setup_us: result.decode_setup_us,
        encoder_setup_us: result.encoder_setup_us,
        encode_time_us: result.encode_time_us,
        total_encode_us: result.total_encode_us,
        output_size: result.output_size,
        num_points: result.num_points,
        num_faces: result.num_faces,
        num_attributes: result.num_attributes,
    })
}

#[cfg(cpp_test_bridge_disabled)]
pub fn profile_cpp_reencode_mesh(
    _encoded_data: &[u8],
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
    _iterations: u32,
) -> Option<CppReencodeProfileResult> {
    None
}

/// Decode profiling result from C++
#[derive(Debug, Clone)]
pub struct CppDecodeProfileResult {
    pub decode_time_us: i64,
    pub num_points: u32,
    pub num_faces: u32,
}

/// Structural and data fingerprint for a decoded C++ mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CppDecodeFingerprint {
    pub num_points: u32,
    pub num_faces: u32,
    pub num_attributes: u32,
    pub face_hash: u64,
    pub attribute_hash: u64,
    pub canonical_corner_hash: u64,
}

/// Profile C++ decoding
#[cfg(not(cpp_test_bridge_disabled))]
pub fn profile_cpp_decode(encoded_data: &[u8], iterations: u32) -> Option<CppDecodeProfileResult> {
    let mut result = ffi::DracoDecodeProfileResult {
        decode_time_us: 0,
        num_points: 0,
        num_faces: 0,
    };

    let status = unsafe {
        ffi::draco_profile_decode(
            encoded_data.as_ptr(),
            encoded_data.len(),
            iterations,
            &mut result,
        )
    };

    if status != 0 {
        return None;
    }

    Some(CppDecodeProfileResult {
        decode_time_us: result.decode_time_us,
        num_points: result.num_points,
        num_faces: result.num_faces,
    })
}

#[cfg(cpp_test_bridge_disabled)]
pub fn profile_cpp_decode(
    _encoded_data: &[u8],
    _iterations: u32,
) -> Option<CppDecodeProfileResult> {
    None
}

/// Decode a mesh with C++ Draco and return stable fingerprints for comparison.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn decode_cpp_mesh_fingerprint(encoded_data: &[u8]) -> Option<CppDecodeFingerprint> {
    let mut result = ffi::DracoDecodeFingerprint {
        num_points: 0,
        num_faces: 0,
        num_attributes: 0,
        face_hash: 0,
        attribute_hash: 0,
        canonical_corner_hash: 0,
    };

    let status = unsafe {
        ffi::draco_decode_mesh_fingerprint(encoded_data.as_ptr(), encoded_data.len(), &mut result)
    };

    if status != 0 {
        return None;
    }

    Some(CppDecodeFingerprint {
        num_points: result.num_points,
        num_faces: result.num_faces,
        num_attributes: result.num_attributes,
        face_hash: result.face_hash,
        attribute_hash: result.attribute_hash,
        canonical_corner_hash: result.canonical_corner_hash,
    })
}

#[cfg(cpp_test_bridge_disabled)]
pub fn decode_cpp_mesh_fingerprint(_encoded_data: &[u8]) -> Option<CppDecodeFingerprint> {
    None
}

/// Decode a point cloud with C++ Draco and return stable fingerprints for comparison.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn decode_cpp_point_cloud_fingerprint(encoded_data: &[u8]) -> Option<CppDecodeFingerprint> {
    let mut result = ffi::DracoDecodeFingerprint {
        num_points: 0,
        num_faces: 0,
        num_attributes: 0,
        face_hash: 0,
        attribute_hash: 0,
        canonical_corner_hash: 0,
    };

    let status = unsafe {
        ffi::draco_decode_point_cloud_fingerprint(
            encoded_data.as_ptr(),
            encoded_data.len(),
            &mut result,
        )
    };

    if status != 0 {
        return None;
    }

    Some(CppDecodeFingerprint {
        num_points: result.num_points,
        num_faces: result.num_faces,
        num_attributes: result.num_attributes,
        face_hash: result.face_hash,
        attribute_hash: result.attribute_hash,
        canonical_corner_hash: result.canonical_corner_hash,
    })
}

#[cfg(cpp_test_bridge_disabled)]
pub fn decode_cpp_point_cloud_fingerprint(_encoded_data: &[u8]) -> Option<CppDecodeFingerprint> {
    None
}

/// Benchmark C++ decoding via the Rust wrapper.
///
/// Returns the median per-iteration decode time in nanoseconds and output sizes.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn benchmark_cpp_decode(encoded_data: &[u8], iterations: u32) -> Option<(i64, u32, u32)> {
    let mut out_num_points = 0u32;
    let mut out_num_faces = 0u32;
    let median_ns = unsafe {
        ffi::draco_benchmark_decode_mesh(
            encoded_data.as_ptr(),
            encoded_data.len(),
            iterations,
            &mut out_num_points as *mut u32,
            &mut out_num_faces as *mut u32,
        )
    };

    if median_ns < 0 {
        None
    } else {
        Some((median_ns, out_num_points, out_num_faces))
    }
}

#[cfg(cpp_test_bridge_disabled)]
pub fn benchmark_cpp_decode(_encoded_data: &[u8], _iterations: u32) -> Option<(i64, u32, u32)> {
    None
}

// --- Safe RAII wrappers for C++ handles -------------------------------------------------

#[cfg(not(cpp_test_bridge_disabled))]
/// RAII wrapper around a C++ Mesh handle
pub struct CppMesh {
    handle: *mut ::std::ffi::c_void,
}

#[cfg(not(cpp_test_bridge_disabled))]
impl CppMesh {
    pub fn new() -> Option<Self> {
        let h = unsafe { draco_create_mesh() };
        if h.is_null() {
            None
        } else {
            Some(CppMesh { handle: h })
        }
    }

    pub fn set_num_faces(&mut self, num_faces: u32) {
        unsafe { draco_mesh_set_num_faces(self.handle, num_faces) }
    }

    pub fn set_face(&mut self, face_idx: u32, v0: u32, v1: u32, v2: u32) {
        unsafe { draco_mesh_set_face(self.handle, face_idx, v0, v1, v2) }
    }

    pub fn add_position_attribute(&mut self, num_points: u32, positions: &[f32]) -> Option<i32> {
        let ret = unsafe {
            draco_mesh_add_position_attribute(self.handle, num_points, positions.as_ptr())
        };
        if ret < 0 {
            None
        } else {
            Some(ret as i32)
        }
    }
}

#[cfg(not(cpp_test_bridge_disabled))]
impl Drop for CppMesh {
    fn drop(&mut self) {
        unsafe { draco_free_mesh(self.handle) }
    }
}

#[cfg(cpp_test_bridge_disabled)]
/// Stub when the C++ test bridge is disabled
pub struct CppMesh;

#[cfg(cpp_test_bridge_disabled)]
impl CppMesh {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn set_num_faces(&mut self, _num_faces: u32) {}

    pub fn set_face(&mut self, _face_idx: u32, _v0: u32, _v1: u32, _v2: u32) {}

    pub fn add_position_attribute(&mut self, _num_points: u32, _positions: &[f32]) -> Option<i32> {
        None
    }
}

#[cfg(not(cpp_test_bridge_disabled))]
/// RAII wrapper around a C++ EncoderBuffer handle
pub struct CppEncoderBuffer {
    handle: *mut ::std::ffi::c_void,
}

#[cfg(not(cpp_test_bridge_disabled))]
impl CppEncoderBuffer {
    pub fn new() -> Option<Self> {
        let h = unsafe { draco_create_encoder_buffer() };
        if h.is_null() {
            None
        } else {
            Some(CppEncoderBuffer { handle: h })
        }
    }

    pub fn data(&self) -> &[u8] {
        unsafe {
            let ptr = draco_encoder_buffer_data(self.handle);
            let len = draco_encoder_buffer_size(self.handle);
            std::slice::from_raw_parts(ptr, len)
        }
    }
}

#[cfg(not(cpp_test_bridge_disabled))]
impl Drop for CppEncoderBuffer {
    fn drop(&mut self) {
        unsafe { draco_free_encoder_buffer(self.handle) }
    }
}

#[cfg(cpp_test_bridge_disabled)]
pub struct CppEncoderBuffer;

#[cfg(cpp_test_bridge_disabled)]
impl CppEncoderBuffer {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn data(&self) -> &[u8] {
        &[]
    }
}

/// Encode using the C++ handle-based API and return encoded bytes
#[cfg(not(cpp_test_bridge_disabled))]
pub fn encode_with_handles(
    mesh: &CppMesh,
    encoding_speed: i32,
    decoding_speed: i32,
    quantization_bits: i32,
) -> Option<Vec<u8>> {
    let buffer = CppEncoderBuffer::new()?;
    let status = unsafe {
        draco_encode_mesh(
            mesh.handle,
            buffer.handle,
            encoding_speed,
            decoding_speed,
            quantization_bits,
        )
    };
    if status < 0 {
        return None;
    }
    Some(buffer.data().to_vec())
}

#[cfg(cpp_test_bridge_disabled)]
pub fn encode_with_handles(
    _mesh: &CppMesh,
    _encoding_speed: i32,
    _decoding_speed: i32,
    _quantization_bits: i32,
) -> Option<Vec<u8>> {
    None
}

// --------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpp_test_bridge_available() {
        if is_available() {
            let (major, minor, revision) = get_version();
            println!("Draco C++ version: {}.{}.{}", major, minor, revision);
        } else {
            println!("C++ test bridge is disabled");
        }
    }
}

/// One value per point is the usual case and the one
/// [`encode_cpp_mesh_attributed`] covers. This is the other one: `position_map`
/// and `uv_map` say which value each point uses, so a vertex shared by two
/// faces can carry a different UV in each, and the encoder has to split the
/// attribute's connectivity from the position's.
#[cfg(not(cpp_test_bridge_disabled))]
#[allow(clippy::too_many_arguments)]
pub fn encode_cpp_mesh_seamed(
    positions: &[f32],
    position_map: &[u32],
    uvs: &[f32],
    uv_map: &[u32],
    faces: &[u32],
    encoding_speed: i32,
    decoding_speed: i32,
    position_bits: i32,
    uv_bits: i32,
) -> Option<Vec<u8>> {
    let num_points = position_map.len() as u32;
    let num_faces = (faces.len() / 3) as u32;
    let buffer_size = (num_points as usize * 32 + faces.len() * 4 + 4096).max(65536);
    let mut buffer = vec![0u8; buffer_size];

    let encoded_size = unsafe {
        ffi::draco_encode_mesh_seamed(
            num_points,
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            position_map.as_ptr(),
            (uvs.len() / 2) as u32,
            uvs.as_ptr(),
            uv_map.as_ptr(),
            num_faces,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            position_bits,
            uv_bits,
            buffer.as_mut_ptr(),
            buffer_size,
        )
    };

    if encoded_size == 0 {
        return None;
    }
    buffer.truncate(encoded_size);
    Some(buffer)
}

#[cfg(cpp_test_bridge_disabled)]
#[allow(clippy::too_many_arguments)]
pub fn encode_cpp_mesh_seamed(
    _positions: &[f32],
    _position_map: &[u32],
    _uvs: &[f32],
    _uv_map: &[u32],
    _faces: &[u32],
    _encoding_speed: i32,
    _decoding_speed: i32,
    _position_bits: i32,
    _uv_bits: i32,
) -> Option<Vec<u8>> {
    None
}
