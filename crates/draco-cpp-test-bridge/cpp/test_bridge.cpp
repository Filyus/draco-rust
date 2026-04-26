// Internal C++ test bridge for Rust parity and performance tests.
// This is not a public C API surface.

#include <cstring>
#include <cstdint>
#include <chrono>
#include <algorithm>
#include <vector>

#include "draco/compression/encode.h"
#include "draco/compression/decode.h"
#include "draco/mesh/mesh.h"
#include "draco/mesh/triangle_soup_mesh_builder.h"
#include "draco/point_cloud/point_cloud.h"
#include "draco/core/encoder_buffer.h"
#include "draco/core/decoder_buffer.h"

extern "C" {

static int64_t rounded_ns_to_us(int64_t ns) {
    return (ns + 500) / 1000;
}

// Opaque handle types
typedef void* DracoMeshHandle;
typedef void* DracoEncoderBufferHandle;

// Create a new mesh
DracoMeshHandle draco_create_mesh() {
    return new draco::Mesh();
}

// Free a mesh
void draco_free_mesh(DracoMeshHandle handle) {
    delete static_cast<draco::Mesh*>(handle);
}

// Set mesh face count
void draco_mesh_set_num_faces(DracoMeshHandle handle, uint32_t num_faces) {
    auto* mesh = static_cast<draco::Mesh*>(handle);
    mesh->SetNumFaces(num_faces);
}

// Add a face to the mesh
void draco_mesh_set_face(DracoMeshHandle handle, uint32_t face_idx, uint32_t v0, uint32_t v1, uint32_t v2) {
    auto* mesh = static_cast<draco::Mesh*>(handle);
    draco::Mesh::Face face;
    face[0] = draco::PointIndex(v0);
    face[1] = draco::PointIndex(v1);
    face[2] = draco::PointIndex(v2);
    mesh->SetFace(draco::FaceIndex(face_idx), face);
}

// Set number of points and add position attribute
int draco_mesh_add_position_attribute(DracoMeshHandle handle, uint32_t num_points, const float* positions) {
    auto* mesh = static_cast<draco::Mesh*>(handle);

    // Create a GeometryAttribute with explicit stride/offset to match single-shot construction
    draco::GeometryAttribute ga;
    ga.Init(draco::GeometryAttribute::POSITION, nullptr, 3, draco::DT_FLOAT32, false, sizeof(float) * 3, 0);

    int pos_att_id = mesh->AddAttribute(ga, true, num_points);
    if (pos_att_id < 0) return -1;
    draco::PointAttribute* pos_att = mesh->attribute(pos_att_id);

    for (uint32_t i = 0; i < num_points; ++i) {
        pos_att->SetAttributeValue(draco::AttributeValueIndex(i), &positions[i * 3]);
    }

    mesh->set_num_points(num_points);
    return pos_att_id;
}

// Create encoder buffer
DracoEncoderBufferHandle draco_create_encoder_buffer() {
    return new draco::EncoderBuffer();
}

// Free encoder buffer
void draco_free_encoder_buffer(DracoEncoderBufferHandle handle) {
    delete static_cast<draco::EncoderBuffer*>(handle);
}

// Get encoded data pointer and size
const uint8_t* draco_encoder_buffer_data(DracoEncoderBufferHandle handle) {
    auto* buffer = static_cast<draco::EncoderBuffer*>(handle);
    return reinterpret_cast<const uint8_t*>(buffer->data());
}

size_t draco_encoder_buffer_size(DracoEncoderBufferHandle handle) {
    auto* buffer = static_cast<draco::EncoderBuffer*>(handle);
    return buffer->size();
}

// Encode mesh with given speed and quantization settings
// Returns encoding time in microseconds, or -1 on error
int64_t draco_encode_mesh(
    DracoMeshHandle mesh_handle,
    DracoEncoderBufferHandle buffer_handle,
    int encoding_speed,
    int decoding_speed,
    int quantization_bits
) {
    auto* mesh = static_cast<draco::Mesh*>(mesh_handle);
    auto* buffer = static_cast<draco::EncoderBuffer*>(buffer_handle);
    
    draco::Encoder encoder;
    encoder.SetSpeedOptions(encoding_speed, decoding_speed);
    encoder.SetAttributeQuantization(draco::GeometryAttribute::POSITION, quantization_bits);
    // Don't set encoding method - let C++ use default (sequential at speed 10, edgebreaker otherwise)
    
    auto start = std::chrono::steady_clock::now();
    draco::Status status = encoder.EncodeMeshToBuffer(*mesh, buffer);
    auto end = std::chrono::steady_clock::now();
    
    if (!status.ok()) {
        return -1;
    }
    
    auto duration = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start);
    return rounded_ns_to_us(duration.count());
}

// Benchmark encoding: runs encoding multiple times and returns average time in microseconds
// Uses direct mesh construction to match Rust's mesh structure exactly
int64_t draco_benchmark_encode_mesh(
    uint32_t num_points,
    const float* positions,
    uint32_t num_faces,
    const uint32_t* faces,  // Each face is 3 consecutive indices
    int encoding_speed,
    int decoding_speed,
    int quantization_bits,
    uint32_t iterations,
    size_t* output_size  // Output: encoded size in bytes
) {
    if (iterations == 0) {
        return -1;
    }

    int64_t total_time_ns = 0;
    *output_size = 0;
    
    for (uint32_t iter = 0; iter < iterations; ++iter) {
        // Create mesh directly (matching Rust's approach)
        draco::Mesh mesh;
        mesh.set_num_points(num_points);
        mesh.SetNumFaces(num_faces);
        
        // Create position attribute with explicit identity mapping
        draco::GeometryAttribute ga;
        ga.Init(draco::GeometryAttribute::POSITION, nullptr, 3, draco::DT_FLOAT32, 
                false, sizeof(float) * 3, 0);
        
        // AddAttribute with identity_mapping = true creates proper identity mapped attribute
        int pos_att_id = mesh.AddAttribute(ga, true, num_points);
        draco::PointAttribute* pos_att = mesh.attribute(pos_att_id);
        
        // Set attribute values
        for (uint32_t i = 0; i < num_points; ++i) {
            pos_att->SetAttributeValue(draco::AttributeValueIndex(i), &positions[i * 3]);
        }
        
        // Set faces
        for (uint32_t i = 0; i < num_faces; ++i) {
            draco::Mesh::Face face;
            face[0] = draco::PointIndex(faces[i * 3]);
            face[1] = draco::PointIndex(faces[i * 3 + 1]);
            face[2] = draco::PointIndex(faces[i * 3 + 2]);
            mesh.SetFace(draco::FaceIndex(i), face);
        }
        
        // Setup encoder
        draco::Encoder encoder;
        encoder.SetSpeedOptions(encoding_speed, decoding_speed);
        encoder.SetAttributeQuantization(draco::GeometryAttribute::POSITION, quantization_bits);
        
        draco::EncoderBuffer buffer;
        
        // Time just the encoding
        auto start = std::chrono::steady_clock::now();
        draco::Status status = encoder.EncodeMeshToBuffer(mesh, &buffer);
        auto end = std::chrono::steady_clock::now();
        
        if (!status.ok()) {
            return -1;
        }
        
        auto duration = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start);
        total_time_ns += duration.count();
        *output_size = buffer.size();
    }
    
    return rounded_ns_to_us(total_time_ns / iterations);
}

// Get version info for verification
void draco_get_version(int* major, int* minor, int* revision) {
    // Draco version from CMakeLists.txt
    *major = 1;
    *minor = 5;
    *revision = 7;
}

// Profiling result structure
struct DracoProfileResult {
    int64_t mesh_setup_us;      // Time to create mesh and set attributes
    int64_t encoder_setup_us;   // Time to create and configure encoder
    int64_t encode_time_us;     // Time for actual encoding
    int64_t total_time_us;      // Total time including mesh setup
    size_t output_size;
};

// Detailed profiling of encoding stages
// Returns 0 on success, -1 on error
int draco_profile_encode(
    uint32_t num_points,
    const float* positions,
    uint32_t num_faces,
    const uint32_t* faces,
    int encoding_speed,
    int decoding_speed,
    int quantization_bits,
    uint32_t iterations,
    DracoProfileResult* result
) {
    if (iterations == 0) {
        return -1;
    }

    int64_t total_mesh_setup_ns = 0;
    int64_t total_encoder_setup_ns = 0;
    int64_t total_encode_ns = 0;
    int64_t total_all_ns = 0;
    
    for (uint32_t iter = 0; iter < iterations; ++iter) {
        auto all_start = std::chrono::steady_clock::now();
        
        // === MESH SETUP ===
        auto mesh_start = std::chrono::steady_clock::now();
        
        draco::Mesh mesh;
        mesh.set_num_points(num_points);
        mesh.SetNumFaces(num_faces);
        
        draco::GeometryAttribute ga;
        ga.Init(draco::GeometryAttribute::POSITION, nullptr, 3, draco::DT_FLOAT32, 
                false, sizeof(float) * 3, 0);
        
        int pos_att_id = mesh.AddAttribute(ga, true, num_points);
        draco::PointAttribute* pos_att = mesh.attribute(pos_att_id);
        
        for (uint32_t i = 0; i < num_points; ++i) {
            pos_att->SetAttributeValue(draco::AttributeValueIndex(i), &positions[i * 3]);
        }
        
        for (uint32_t i = 0; i < num_faces; ++i) {
            draco::Mesh::Face face;
            face[0] = draco::PointIndex(faces[i * 3]);
            face[1] = draco::PointIndex(faces[i * 3 + 1]);
            face[2] = draco::PointIndex(faces[i * 3 + 2]);
            mesh.SetFace(draco::FaceIndex(i), face);
        }
        
        auto mesh_end = std::chrono::steady_clock::now();
        
        // === ENCODER SETUP ===
        auto encoder_start = std::chrono::steady_clock::now();
        
        draco::Encoder encoder;
        encoder.SetSpeedOptions(encoding_speed, decoding_speed);
        encoder.SetAttributeQuantization(draco::GeometryAttribute::POSITION, quantization_bits);
        draco::EncoderBuffer buffer;
        
        auto encoder_end = std::chrono::steady_clock::now();
        
        // === ENCODING ===
        auto encode_start = std::chrono::steady_clock::now();
        draco::Status status = encoder.EncodeMeshToBuffer(mesh, &buffer);
        auto encode_end = std::chrono::steady_clock::now();
        
        if (!status.ok()) {
            return -1;
        }
        
        auto all_end = std::chrono::steady_clock::now();
        
        total_mesh_setup_ns += std::chrono::duration_cast<std::chrono::nanoseconds>(mesh_end - mesh_start).count();
        total_encoder_setup_ns += std::chrono::duration_cast<std::chrono::nanoseconds>(encoder_end - encoder_start).count();
        total_encode_ns += std::chrono::duration_cast<std::chrono::nanoseconds>(encode_end - encode_start).count();
        total_all_ns += std::chrono::duration_cast<std::chrono::nanoseconds>(all_end - all_start).count();
        
        result->output_size = buffer.size();
    }
    
    result->mesh_setup_us = rounded_ns_to_us(total_mesh_setup_ns / iterations);
    result->encoder_setup_us = rounded_ns_to_us(total_encoder_setup_ns / iterations);
    result->encode_time_us = rounded_ns_to_us(total_encode_ns / iterations);
    result->total_time_us = rounded_ns_to_us(total_all_ns / iterations);
    
    return 0;
}

// Single-shot encoding that returns encoded data for byte comparison
// Uses direct mesh construction to match Rust's mesh structure
// Returns encoded size, or 0 on error. Caller provides output buffer.
size_t draco_encode_mesh_single(
    uint32_t num_points,
    const float* positions,
    uint32_t num_faces,
    const uint32_t* faces,
    int encoding_speed,
    int decoding_speed,
    int quantization_bits,
    uint8_t* output_buffer,
    size_t output_buffer_size
) {
    // Create mesh directly (matching Rust's approach)
    draco::Mesh mesh;
    mesh.set_num_points(num_points);
    mesh.SetNumFaces(num_faces);
    
    // Create position attribute with explicit identity mapping
    draco::GeometryAttribute ga;
    ga.Init(draco::GeometryAttribute::POSITION, nullptr, 3, draco::DT_FLOAT32, 
            false, sizeof(float) * 3, 0);
    
    // AddAttribute with identity_mapping = true creates proper identity mapped attribute
    int pos_att_id = mesh.AddAttribute(ga, true, num_points);
    draco::PointAttribute* pos_att = mesh.attribute(pos_att_id);
    
    // Set attribute values
    for (uint32_t i = 0; i < num_points; ++i) {
        pos_att->SetAttributeValue(draco::AttributeValueIndex(i), &positions[i * 3]);
    }
    
    // Set faces
    for (uint32_t i = 0; i < num_faces; ++i) {
        draco::Mesh::Face face;
        face[0] = draco::PointIndex(faces[i * 3]);
        face[1] = draco::PointIndex(faces[i * 3 + 1]);
        face[2] = draco::PointIndex(faces[i * 3 + 2]);
        mesh.SetFace(draco::FaceIndex(i), face);
    }
    
    // Setup encoder
    draco::Encoder encoder;
    encoder.SetSpeedOptions(encoding_speed, decoding_speed);
    encoder.SetAttributeQuantization(draco::GeometryAttribute::POSITION, quantization_bits);
    
    draco::EncoderBuffer buffer;
    draco::Status status = encoder.EncodeMeshToBuffer(mesh, &buffer);
    
    if (!status.ok()) {
        return 0;
    }
    
    size_t encoded_size = buffer.size();
    if (encoded_size > output_buffer_size) {
        return 0;  // Buffer too small
    }
    
    std::memcpy(output_buffer, buffer.data(), encoded_size);
    return encoded_size;
}

// Single-shot encoding with explicit sequential mesh connectivity mode.
// When compress_connectivity is non-zero, this writes connectivity_method = 0,
// whose payload stores delta-coded symbols.
size_t draco_encode_mesh_sequential(
    uint32_t num_points,
    const float* positions,
    uint32_t num_faces,
    const uint32_t* faces,
    int encoding_speed,
    int decoding_speed,
    int quantization_bits,
    int compress_connectivity,
    uint8_t* output_buffer,
    size_t output_buffer_size
) {
    draco::Mesh mesh;
    mesh.set_num_points(num_points);
    mesh.SetNumFaces(num_faces);

    draco::GeometryAttribute ga;
    ga.Init(draco::GeometryAttribute::POSITION, nullptr, 3, draco::DT_FLOAT32,
            false, sizeof(float) * 3, 0);

    int pos_att_id = mesh.AddAttribute(ga, true, num_points);
    draco::PointAttribute* pos_att = mesh.attribute(pos_att_id);

    for (uint32_t i = 0; i < num_points; ++i) {
        pos_att->SetAttributeValue(draco::AttributeValueIndex(i), &positions[i * 3]);
    }

    for (uint32_t i = 0; i < num_faces; ++i) {
        draco::Mesh::Face face;
        face[0] = draco::PointIndex(faces[i * 3]);
        face[1] = draco::PointIndex(faces[i * 3 + 1]);
        face[2] = draco::PointIndex(faces[i * 3 + 2]);
        mesh.SetFace(draco::FaceIndex(i), face);
    }

    draco::Encoder encoder;
    encoder.SetEncodingMethod(draco::MESH_SEQUENTIAL_ENCODING);
    encoder.SetSpeedOptions(encoding_speed, decoding_speed);
    encoder.SetAttributeQuantization(draco::GeometryAttribute::POSITION, quantization_bits);
    encoder.options().SetGlobalBool("compress_connectivity", compress_connectivity != 0);

    draco::EncoderBuffer buffer;
    draco::Status status = encoder.EncodeMeshToBuffer(mesh, &buffer);

    if (!status.ok()) {
        return 0;
    }

    size_t encoded_size = buffer.size();
    if (encoded_size > output_buffer_size) {
        return 0;
    }

    std::memcpy(output_buffer, buffer.data(), encoded_size);
    return encoded_size;
}

// Decode profiling result structure
struct DracoDecodeProfileResult {
    int64_t decode_time_us;
    uint32_t num_points;
    uint32_t num_faces;
};

struct DracoDecodeFingerprint {
    uint32_t num_points;
    uint32_t num_faces;
    uint32_t num_attributes;
    uint64_t face_hash;
    uint64_t attribute_hash;
    uint64_t canonical_corner_hash;
};

static void fnv1a_u8(uint64_t* hash, uint8_t value) {
    *hash ^= static_cast<uint64_t>(value);
    *hash *= 1099511628211ULL;
}

static void fnv1a_bytes(uint64_t* hash, const uint8_t* data, size_t size) {
    for (size_t i = 0; i < size; ++i) {
        fnv1a_u8(hash, data[i]);
    }
}

static void fnv1a_u32(uint64_t* hash, uint32_t value) {
    const uint8_t bytes[4] = {
        static_cast<uint8_t>(value & 0xff),
        static_cast<uint8_t>((value >> 8) & 0xff),
        static_cast<uint8_t>((value >> 16) & 0xff),
        static_cast<uint8_t>((value >> 24) & 0xff),
    };
    fnv1a_bytes(hash, bytes, sizeof(bytes));
}

static void fnv1a_u64(uint64_t* hash, uint64_t value) {
    const uint8_t bytes[8] = {
        static_cast<uint8_t>(value & 0xff),
        static_cast<uint8_t>((value >> 8) & 0xff),
        static_cast<uint8_t>((value >> 16) & 0xff),
        static_cast<uint8_t>((value >> 24) & 0xff),
        static_cast<uint8_t>((value >> 32) & 0xff),
        static_cast<uint8_t>((value >> 40) & 0xff),
        static_cast<uint8_t>((value >> 48) & 0xff),
        static_cast<uint8_t>((value >> 56) & 0xff),
    };
    fnv1a_bytes(hash, bytes, sizeof(bytes));
}

static uint64_t hash_mesh_faces(const draco::Mesh& mesh) {
    uint64_t hash = 1469598103934665603ULL;
    fnv1a_u32(&hash, mesh.num_faces());
    for (uint32_t face_id = 0; face_id < mesh.num_faces(); ++face_id) {
        const draco::Mesh::Face& face = mesh.face(draco::FaceIndex(face_id));
        fnv1a_u32(&hash, face[0].value());
        fnv1a_u32(&hash, face[1].value());
        fnv1a_u32(&hash, face[2].value());
    }
    return hash;
}

static uint64_t hash_mesh_attributes(const draco::Mesh& mesh) {
    uint64_t hash = 1469598103934665603ULL;
    fnv1a_u32(&hash, mesh.num_attributes());
    fnv1a_u32(&hash, mesh.num_points());

    for (int att_id = 0; att_id < mesh.num_attributes(); ++att_id) {
        const draco::PointAttribute* att = mesh.attribute(att_id);
        const uint32_t stride = static_cast<uint32_t>(att->byte_stride());
        fnv1a_u32(&hash, static_cast<uint32_t>(att->attribute_type()));
        fnv1a_u32(&hash, static_cast<uint32_t>(att->data_type()));
        fnv1a_u32(&hash, static_cast<uint32_t>(att->num_components()));
        fnv1a_u32(&hash, att->normalized() ? 1u : 0u);
        fnv1a_u32(&hash, stride);
        fnv1a_u64(&hash, static_cast<uint64_t>(att->size()));

        for (uint32_t point_id = 0; point_id < mesh.num_points(); ++point_id) {
            const draco::AttributeValueIndex value_index =
                att->mapped_index(draco::PointIndex(point_id));
            fnv1a_u32(&hash, value_index.value());
            const uint8_t* value = att->GetAddress(value_index);
            fnv1a_bytes(&hash, value, stride);
        }
    }

    return hash;
}

static uint64_t hash_point_cloud_attributes(const draco::PointCloud& point_cloud) {
    uint64_t hash = 1469598103934665603ULL;
    fnv1a_u32(&hash, point_cloud.num_attributes());
    fnv1a_u32(&hash, point_cloud.num_points());

    for (int att_id = 0; att_id < point_cloud.num_attributes(); ++att_id) {
        const draco::PointAttribute* att = point_cloud.attribute(att_id);
        const uint32_t stride = static_cast<uint32_t>(att->byte_stride());
        fnv1a_u32(&hash, static_cast<uint32_t>(att->attribute_type()));
        fnv1a_u32(&hash, static_cast<uint32_t>(att->data_type()));
        fnv1a_u32(&hash, static_cast<uint32_t>(att->num_components()));
        fnv1a_u32(&hash, att->normalized() ? 1u : 0u);
        fnv1a_u32(&hash, stride);
        fnv1a_u64(&hash, static_cast<uint64_t>(att->size()));

        for (uint32_t point_id = 0; point_id < point_cloud.num_points(); ++point_id) {
            const draco::AttributeValueIndex value_index =
                att->mapped_index(draco::PointIndex(point_id));
            fnv1a_u32(&hash, value_index.value());
            const uint8_t* value = att->GetAddress(value_index);
            fnv1a_bytes(&hash, value, stride);
        }
    }

    return hash;
}

static uint64_t hash_mesh_canonical_corners(const draco::Mesh& mesh) {
    uint64_t metadata_hash = 1469598103934665603ULL;
    fnv1a_u32(&metadata_hash, mesh.num_attributes());
    for (int att_id = 0; att_id < mesh.num_attributes(); ++att_id) {
        const draco::PointAttribute* att = mesh.attribute(att_id);
        fnv1a_u32(&metadata_hash, static_cast<uint32_t>(att->attribute_type()));
        fnv1a_u32(&metadata_hash, static_cast<uint32_t>(att->data_type()));
        fnv1a_u32(&metadata_hash, static_cast<uint32_t>(att->num_components()));
        fnv1a_u32(&metadata_hash, att->normalized() ? 1u : 0u);
        fnv1a_u32(&metadata_hash, static_cast<uint32_t>(att->byte_stride()));
    }

    std::vector<uint64_t> face_hashes;
    face_hashes.reserve(mesh.num_faces());

    for (uint32_t face_id = 0; face_id < mesh.num_faces(); ++face_id) {
        uint64_t face_hash = metadata_hash;
        const draco::Mesh::Face& face = mesh.face(draco::FaceIndex(face_id));
        for (int corner = 0; corner < 3; ++corner) {
            for (int att_id = 0; att_id < mesh.num_attributes(); ++att_id) {
                const draco::PointAttribute* att = mesh.attribute(att_id);
                const uint32_t stride = static_cast<uint32_t>(att->byte_stride());
                const draco::AttributeValueIndex value_index = att->mapped_index(face[corner]);
                const uint8_t* value = att->GetAddress(value_index);
                fnv1a_bytes(&face_hash, value, stride);
            }
        }
        face_hashes.push_back(face_hash);
    }

    std::sort(face_hashes.begin(), face_hashes.end());

    uint64_t hash = 1469598103934665603ULL;
    fnv1a_u32(&hash, mesh.num_faces());
    fnv1a_u32(&hash, mesh.num_attributes());
    for (uint64_t face_hash : face_hashes) {
        fnv1a_u64(&hash, face_hash);
    }
    return hash;
}

// Benchmark decoding: runs decoding multiple times and returns the median
// per-iteration decode time in nanoseconds. The setup mirrors the Rust harness:
// DecoderBuffer and Decoder construction are outside the timed region.
int64_t draco_benchmark_decode_mesh(
    const uint8_t* encoded_data,
    size_t encoded_size,
    uint32_t iterations,
    uint32_t* out_num_points,
    uint32_t* out_num_faces
) {
    if (iterations == 0) {
        return -1;
    }

    const uint32_t warmup_iterations = std::min(std::max(iterations, 10u), 50u);
    for (uint32_t iter = 0; iter < warmup_iterations; ++iter) {
        draco::DecoderBuffer buffer;
        buffer.Init(reinterpret_cast<const char*>(encoded_data), encoded_size);

        draco::Decoder decoder;
        auto result = decoder.DecodeMeshFromBuffer(&buffer);
        if (!result.ok()) {
            return -1;
        }

        auto mesh = std::move(result).value();
        *out_num_points = mesh->num_points();
        *out_num_faces = mesh->num_faces();
    }

    constexpr int kBatches = 9;
    std::vector<int64_t> batch_ns;
    batch_ns.reserve(kBatches);

    for (int batch = 0; batch < kBatches; ++batch) {
        int64_t total_ns = 0;

        for (uint32_t iter = 0; iter < iterations; ++iter) {
            draco::DecoderBuffer buffer;
            buffer.Init(reinterpret_cast<const char*>(encoded_data), encoded_size);

            draco::Decoder decoder;

            auto start = std::chrono::steady_clock::now();
            auto result = decoder.DecodeMeshFromBuffer(&buffer);
            auto end = std::chrono::steady_clock::now();

            if (!result.ok()) {
                return -1;
            }

            total_ns += std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count();

            auto mesh = std::move(result).value();
            *out_num_points = mesh->num_points();
            *out_num_faces = mesh->num_faces();
        }

        batch_ns.push_back(total_ns / iterations);
    }

    std::sort(batch_ns.begin(), batch_ns.end());
    return batch_ns[kBatches / 2];
}

// Profile decoding with detailed timing
int draco_profile_decode(
    const uint8_t* encoded_data,
    size_t encoded_size,
    uint32_t iterations,
    DracoDecodeProfileResult* result
) {
    if (iterations == 0) {
        return -1;
    }

    int64_t total_decode_ns = 0;
    
    for (uint32_t iter = 0; iter < iterations; ++iter) {
        draco::DecoderBuffer buffer;
        buffer.Init(reinterpret_cast<const char*>(encoded_data), encoded_size);
        
        draco::Decoder decoder;
        
        auto start = std::chrono::steady_clock::now();
        auto decode_result = decoder.DecodeMeshFromBuffer(&buffer);
        auto end = std::chrono::steady_clock::now();
        
        if (!decode_result.ok()) {
            return -1;
        }
        
        total_decode_ns += std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count();
        
        auto mesh = std::move(decode_result).value();
        result->num_points = mesh->num_points();
        result->num_faces = mesh->num_faces();
    }
    
    const int64_t avg_decode_ns = total_decode_ns / iterations;
    result->decode_time_us = (avg_decode_ns + 500) / 1000;
    return 0;
}

// Decode a mesh once and return stable structural/data fingerprints.
int draco_decode_mesh_fingerprint(
    const uint8_t* encoded_data,
    size_t encoded_size,
    DracoDecodeFingerprint* result
) {
    draco::DecoderBuffer buffer;
    buffer.Init(reinterpret_cast<const char*>(encoded_data), encoded_size);

    draco::Decoder decoder;
    auto decode_result = decoder.DecodeMeshFromBuffer(&buffer);
    if (!decode_result.ok()) {
        return -1;
    }

    auto mesh = std::move(decode_result).value();
    result->num_points = mesh->num_points();
    result->num_faces = mesh->num_faces();
    result->num_attributes = mesh->num_attributes();
    result->face_hash = hash_mesh_faces(*mesh);
    result->attribute_hash = hash_mesh_attributes(*mesh);
    result->canonical_corner_hash = hash_mesh_canonical_corners(*mesh);
    return 0;
}

// Decode a point cloud once and return stable structural/data fingerprints.
int draco_decode_point_cloud_fingerprint(
    const uint8_t* encoded_data,
    size_t encoded_size,
    DracoDecodeFingerprint* result
) {
    draco::DecoderBuffer buffer;
    buffer.Init(reinterpret_cast<const char*>(encoded_data), encoded_size);

    draco::Decoder decoder;
    auto decode_result = decoder.DecodePointCloudFromBuffer(&buffer);
    if (!decode_result.ok()) {
        return -1;
    }

    auto point_cloud = std::move(decode_result).value();
    result->num_points = point_cloud->num_points();
    result->num_faces = 0;
    result->num_attributes = point_cloud->num_attributes();
    result->face_hash = 0;
    result->attribute_hash = hash_point_cloud_attributes(*point_cloud);
    result->canonical_corner_hash = 0;
    return 0;
}

} // extern "C"
