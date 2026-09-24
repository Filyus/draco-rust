//! Internal C++ test bridge for Rust parity and performance tests
//!
//! This crate is not a public C API surface. It provides a private bridge to
//! the original C++ Draco encoder/decoder for Rust parity and performance tests.

pub mod counting;

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

    /// `draco::Mesh`, owned by the C++ side and only ever behind a pointer.
    #[repr(C)]
    pub struct DracoMesh {
        _opaque: [u8; 0],
    }

    /// Bytes the C++ side made for Rust to take; see [`super::CppBytes`].
    #[repr(C)]
    pub struct DracoBytes {
        _opaque: [u8; 0],
    }

    extern "C" {
        pub fn draco_bytes_size(bytes: *const DracoBytes) -> usize;
        pub fn draco_bytes_data(bytes: *const DracoBytes) -> *const u8;
        pub fn draco_bytes_free(bytes: *mut DracoBytes);

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

        pub fn draco_encode_mesh_positions(
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
        ) -> *mut DracoBytes;

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
        ) -> *mut DracoBytes;

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
        ) -> *mut DracoBytes;

        /// Point cloud encoding. `encoding_method` is -1 to leave the choice to
        /// Draco's own selection rule, 0 for sequential, 1 for kd-tree.
        #[allow(clippy::too_many_arguments)]
        pub fn draco_encode_point_cloud(
            num_points: u32,
            positions: *const f32,
            normals: *const f32,
            colors: *const u8,
            encoding_method: c_int,
            encoding_speed: c_int,
            decoding_speed: c_int,
            position_bits: c_int,
            normal_bits: c_int,
            color_bits: c_int,
        ) -> *mut DracoBytes;
        /// A mesh or point cloud carrying one POSITION attribute plus one GENERIC
        /// attribute of an arbitrary `draco::DataType`. `generic_data` is already
        /// packed at `DataTypeLength(generic_data_type) * generic_num_components`
        /// bytes per point. `is_mesh` selects geometry kind; `faces`/`num_faces`
        /// are ignored for a point cloud.
        #[allow(clippy::too_many_arguments)]
        pub fn draco_encode_generic(
            is_mesh: c_int,
            num_points: u32,
            positions: *const f32,
            num_faces: u32,
            faces: *const u32,
            generic_data_type: c_int,
            generic_num_components: c_int,
            generic_data: *const u8,
            generic_quantization_bits: c_int,
            encoding_method: c_int,
            encoding_speed: c_int,
            decoding_speed: c_int,
            position_bits: c_int,
        ) -> *mut DracoBytes;
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
        ) -> *mut DracoBytes;

        pub fn draco_mesh_new() -> *mut DracoMesh;
        pub fn draco_mesh_free(mesh: *mut DracoMesh);
        pub fn draco_mesh_set_num_faces(mesh: *mut DracoMesh, num_faces: u32);
        pub fn draco_mesh_set_face(mesh: *mut DracoMesh, face_idx: u32, v0: u32, v1: u32, v2: u32);
        pub fn draco_mesh_add_position_attribute(
            mesh: *mut DracoMesh,
            num_points: u32,
            positions: *const f32,
        ) -> c_int;
        pub fn draco_mesh_encode(
            mesh: *const DracoMesh,
            encoding_speed: c_int,
            decoding_speed: c_int,
            quantization_bits: c_int,
        ) -> *mut DracoBytes;

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

        /// C++-side allocation counters; zeros unless the bridge was built
        /// with `DRACO_BRIDGE_COUNT_ALLOCS` set.
        pub fn draco_alloc_counters(count: *mut u64, bytes: *mut u64);

        /// Profile decoding with detailed timing
        pub fn draco_profile_decode(
            encoded_data: *const u8,
            encoded_size: usize,
            iterations: u32,
            result: *mut DracoDecodeProfileResult,
        ) -> c_int;

        /// Time `CornerTable::Create` alone on a prebuilt face array.
        pub fn draco_profile_corner_table(
            num_faces: u32,
            faces: *const u32,
            iterations: u32,
            out_us: *mut i64,
            out_num_vertices: *mut u32,
            out_num_degenerated: *mut u32,
        ) -> c_int;

        /// Decode a mesh and return one attribute's values as `f32` bytes.
        pub fn draco_decode_mesh_attribute(
            encoded_data: *const u8,
            encoded_size: usize,
            attribute_type: c_int,
        ) -> *mut DracoBytes;

        /// As `draco_decode_mesh_attribute`, for a point cloud.
        pub fn draco_decode_point_cloud_attribute(
            encoded_data: *const u8,
            encoded_size: usize,
            attribute_type: c_int,
        ) -> *mut DracoBytes;

        /// Decode a mesh once and return stable structural/data fingerprints.
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

/// Bytes the C++ side produced, taken over by Rust.
///
/// The C++ side owns the allocation until [`CppBytes::into_vec`] copies it out,
/// so the result is sized by whoever made it: nothing guesses a capacity in
/// advance, and a null handle is the only failure there is.
#[cfg(not(cpp_test_bridge_disabled))]
struct CppBytes(std::ptr::NonNull<ffi::DracoBytes>);

#[cfg(not(cpp_test_bridge_disabled))]
impl CppBytes {
    /// Takes ownership of a handle a bridge function returned; `None` for null.
    fn take(bytes: *mut ffi::DracoBytes) -> Option<Self> {
        std::ptr::NonNull::new(bytes).map(Self)
    }

    fn into_vec(self) -> Vec<u8> {
        unsafe {
            let size = ffi::draco_bytes_size(self.0.as_ptr());
            if size == 0 {
                return Vec::new();
            }
            std::slice::from_raw_parts(ffi::draco_bytes_data(self.0.as_ptr()), size).to_vec()
        }
    }

    fn into_f32s(self) -> Vec<f32> {
        let bytes = self.into_vec();
        let (values, _) = bytes.as_chunks::<4>();
        values.iter().map(|b| f32::from_ne_bytes(*b)).collect()
    }
}

#[cfg(not(cpp_test_bridge_disabled))]
impl Drop for CppBytes {
    fn drop(&mut self) {
        unsafe { ffi::draco_bytes_free(self.0.as_ptr()) }
    }
}

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

/// The C++ Draco version the bridge reports, as `(major, minor, revision)`.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn cpp_version() -> (i32, i32, i32) {
    let mut major = 0;
    let mut minor = 0;
    let mut revision = 0;
    unsafe {
        ffi::draco_get_version(&mut major, &mut minor, &mut revision);
    }
    (major, minor, revision)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn cpp_version() -> (i32, i32, i32) {
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
        ffi::draco_benchmark_encode_mesh(
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

/// C++-side allocation counters since process start: `(count, bytes)`.
///
/// Zeros unless the bridge was built with `DRACO_BRIDGE_COUNT_ALLOCS` set --
/// a counting build pays an atomic per new/delete, so it exists for counting
/// runs only, never timing.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn cpp_alloc_counters() -> (u64, u64) {
    let mut count = 0u64;
    let mut bytes = 0u64;
    unsafe { ffi::draco_alloc_counters(&mut count, &mut bytes) };
    (count, bytes)
}

/// Stub for builds without the C++ Draco library.
#[cfg(cpp_test_bridge_disabled)]
pub fn cpp_alloc_counters() -> (u64, u64) {
    (0, 0)
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
    let bytes = unsafe {
        ffi::draco_encode_mesh_positions(
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            (faces.len() / 3) as u32,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
        )
    };
    CppBytes::take(bytes).map(CppBytes::into_vec)
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
///
/// Values come per point, `num_components` floats each. `None` when the
/// payload does not decode as a mesh or has no attribute of that type.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn decode_cpp_mesh_attribute(encoded: &[u8], attribute_type: i32) -> Option<Vec<f32>> {
    let bytes = unsafe {
        ffi::draco_decode_mesh_attribute(encoded.as_ptr(), encoded.len(), attribute_type)
    };
    CppBytes::take(bytes).map(CppBytes::into_f32s)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn decode_cpp_mesh_attribute(_encoded: &[u8], _attribute_type: i32) -> Option<Vec<f32>> {
    None
}

/// As [`decode_cpp_mesh_attribute`], for a payload holding a point cloud.
///
/// The two entry points differ only in which `Decode*FromBuffer` they call,
/// and a point cloud decoded as a mesh fails outright, so the caller picks.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn decode_cpp_point_cloud_attribute(encoded: &[u8], attribute_type: i32) -> Option<Vec<f32>> {
    let bytes = unsafe {
        ffi::draco_decode_point_cloud_attribute(encoded.as_ptr(), encoded.len(), attribute_type)
    };
    CppBytes::take(bytes).map(CppBytes::into_f32s)
}

#[cfg(cpp_test_bridge_disabled)]
pub fn decode_cpp_point_cloud_attribute(_encoded: &[u8], _attribute_type: i32) -> Option<Vec<f32>> {
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
    let bytes = unsafe {
        ffi::draco_encode_mesh_attributed(
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            (faces.len() / 3) as u32,
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
        )
    };
    CppBytes::take(bytes).map(CppBytes::into_vec)
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
    let bytes = unsafe {
        ffi::draco_encode_mesh_sequential(
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            (faces.len() / 3) as u32,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            quantization_bits,
            i32::from(compress_connectivity),
        )
    };
    CppBytes::take(bytes).map(CppBytes::into_vec)
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

/// C++ corner-table construction time, in microseconds per `CornerTable::Create`.
///
/// `faces` is one `u32` vertex index per corner, three per face -- the same
/// flat run `CornerTable::init` takes on the Rust side, so the two are timed
/// on identical input. Returns `(us, num_vertices, num_degenerated_faces)`;
/// the two counts are there to check both sides built the same table.
#[cfg(not(cpp_test_bridge_disabled))]
pub fn profile_cpp_corner_table(faces: &[u32], iterations: u32) -> Option<(i64, u32, u32)> {
    let num_faces = (faces.len() / 3) as u32;
    let mut us = 0i64;
    let mut num_vertices = 0u32;
    let mut num_degenerated = 0u32;
    let status = unsafe {
        ffi::draco_profile_corner_table(
            num_faces,
            faces.as_ptr(),
            iterations,
            &mut us,
            &mut num_vertices,
            &mut num_degenerated,
        )
    };
    if status != 0 {
        return None;
    }
    Some((us, num_vertices, num_degenerated))
}

#[cfg(cpp_test_bridge_disabled)]
pub fn profile_cpp_corner_table(_faces: &[u32], _iterations: u32) -> Option<(i64, u32, u32)> {
    None
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

/// A C++ `draco::Mesh` assembled one call at a time, the way an application
/// builds one, rather than from the flat arrays the `encode_cpp_*` functions
/// take.
#[cfg(not(cpp_test_bridge_disabled))]
pub struct CppMesh {
    mesh: std::ptr::NonNull<ffi::DracoMesh>,
}

#[cfg(not(cpp_test_bridge_disabled))]
impl CppMesh {
    pub fn new() -> Option<Self> {
        let mesh = std::ptr::NonNull::new(unsafe { ffi::draco_mesh_new() })?;
        Some(CppMesh { mesh })
    }

    pub fn set_num_faces(&mut self, num_faces: u32) {
        unsafe { ffi::draco_mesh_set_num_faces(self.mesh.as_ptr(), num_faces) }
    }

    pub fn set_face(&mut self, face_idx: u32, v0: u32, v1: u32, v2: u32) {
        unsafe { ffi::draco_mesh_set_face(self.mesh.as_ptr(), face_idx, v0, v1, v2) }
    }

    pub fn add_position_attribute(&mut self, num_points: u32, positions: &[f32]) -> Option<i32> {
        let ret = unsafe {
            ffi::draco_mesh_add_position_attribute(
                self.mesh.as_ptr(),
                num_points,
                positions.as_ptr(),
            )
        };
        if ret < 0 {
            None
        } else {
            Some(ret as i32)
        }
    }

    /// Encodes the mesh, leaving the encoding method to Draco.
    pub fn encode(
        &self,
        encoding_speed: i32,
        decoding_speed: i32,
        quantization_bits: i32,
    ) -> Option<Vec<u8>> {
        let bytes = unsafe {
            ffi::draco_mesh_encode(
                self.mesh.as_ptr(),
                encoding_speed,
                decoding_speed,
                quantization_bits,
            )
        };
        CppBytes::take(bytes).map(CppBytes::into_vec)
    }
}

#[cfg(not(cpp_test_bridge_disabled))]
impl Drop for CppMesh {
    fn drop(&mut self) {
        unsafe { ffi::draco_mesh_free(self.mesh.as_ptr()) }
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

    pub fn encode(
        &self,
        _encoding_speed: i32,
        _decoding_speed: i32,
        _quantization_bits: i32,
    ) -> Option<Vec<u8>> {
        None
    }
}

// --------------------------------------------------------------------------------------

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
    let bytes = unsafe {
        ffi::draco_encode_mesh_seamed(
            position_map.len() as u32,
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            position_map.as_ptr(),
            (uvs.len() / 2) as u32,
            uvs.as_ptr(),
            uv_map.as_ptr(),
            (faces.len() / 3) as u32,
            faces.as_ptr(),
            encoding_speed,
            decoding_speed,
            position_bits,
            uv_bits,
        )
    };
    CppBytes::take(bytes).map(CppBytes::into_vec)
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

/// Attributes a point cloud may carry, alongside their quantization.
pub struct CppPointCloudAttributes<'a> {
    pub normals: Option<&'a [f32]>,
    pub colors: Option<&'a [u8]>,
    pub normal_bits: i32,
    pub color_bits: i32,
}

/// Encodes a point cloud with C++ Draco.
///
/// `encoding_method` of `None` leaves the choice to Draco, which is the case
/// worth testing: the selection rule picks kd-tree for a quantized cloud at any
/// speed below 10, and getting that wrong changes the whole payload.
#[cfg(not(cpp_test_bridge_disabled))]
#[allow(clippy::too_many_arguments)]
pub fn encode_cpp_point_cloud(
    positions: &[f32],
    attributes: CppPointCloudAttributes<'_>,
    encoding_method: Option<i32>,
    encoding_speed: i32,
    decoding_speed: i32,
    position_bits: i32,
) -> Option<Vec<u8>> {
    let bytes = unsafe {
        ffi::draco_encode_point_cloud(
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            attributes
                .normals
                .map_or(std::ptr::null(), |values| values.as_ptr()),
            attributes
                .colors
                .map_or(std::ptr::null(), |values| values.as_ptr()),
            encoding_method.unwrap_or(-1),
            encoding_speed,
            decoding_speed,
            position_bits,
            attributes.normal_bits,
            attributes.color_bits,
        )
    };
    CppBytes::take(bytes).map(CppBytes::into_vec)
}

#[cfg(cpp_test_bridge_disabled)]
#[allow(clippy::too_many_arguments)]
pub fn encode_cpp_point_cloud(
    _positions: &[f32],
    _attributes: CppPointCloudAttributes<'_>,
    _encoding_method: Option<i32>,
    _encoding_speed: i32,
    _decoding_speed: i32,
    _position_bits: i32,
) -> Option<Vec<u8>> {
    None
}

/// A generic attribute's raw bytes plus the `draco::DataType` tag they were
/// packed for, so the C++ side reconstructs the same layout without a second
/// encoding of the type.
pub struct CppGenericAttribute<'a> {
    pub data_type: i32,
    pub num_components: i32,
    pub bytes: &'a [u8],
    pub quantization_bits: i32,
}

/// Encodes a mesh or point cloud carrying one POSITION attribute plus one
/// GENERIC attribute of an arbitrary data type, with C++ Draco. Covers what
/// the other bridge functions cannot reach: application-defined attributes,
/// and scalar widths other than `Float32`/`Uint8` (`Int64`/`Uint64`/`Float64`).
#[cfg(not(cpp_test_bridge_disabled))]
#[allow(clippy::too_many_arguments)]
pub fn encode_cpp_generic(
    is_mesh: bool,
    positions: &[f32],
    faces: &[u32],
    attribute: CppGenericAttribute<'_>,
    encoding_method: Option<i32>,
    encoding_speed: i32,
    decoding_speed: i32,
    position_bits: i32,
) -> Option<Vec<u8>> {
    let bytes = unsafe {
        ffi::draco_encode_generic(
            i32::from(is_mesh),
            (positions.len() / 3) as u32,
            positions.as_ptr(),
            (faces.len() / 3) as u32,
            faces.as_ptr(),
            attribute.data_type,
            attribute.num_components,
            attribute.bytes.as_ptr(),
            attribute.quantization_bits,
            encoding_method.unwrap_or(-1),
            encoding_speed,
            decoding_speed,
            position_bits,
        )
    };
    CppBytes::take(bytes).map(CppBytes::into_vec)
}

#[cfg(cpp_test_bridge_disabled)]
#[allow(clippy::too_many_arguments)]
pub fn encode_cpp_generic(
    _is_mesh: bool,
    _positions: &[f32],
    _faces: &[u32],
    _attribute: CppGenericAttribute<'_>,
    _encoding_method: Option<i32>,
    _encoding_speed: i32,
    _decoding_speed: i32,
    _position_bits: i32,
) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpp_test_bridge_available() {
        if is_available() {
            let (major, minor, revision) = cpp_version();
            println!("Draco C++ version: {}.{}.{}", major, minor, revision);
            // Every release is at least 0.9.1, so all zeros is a parse failure.
            assert_ne!((major, minor, revision), (0, 0, 0));
        } else {
            println!("C++ test bridge is disabled");
        }
    }
}
