// Encodes or decodes a synthetic point cloud N times and nothing else -- the
// C++ half of the point-cloud callgrind comparison, matching
// examples/pointcloud_drc.rs step for step, including the generator.
//
// The KD-tree path is why it exists: no mesh driver reaches that encoder, so
// nothing on either side counted it.
//
//   g++ -O3 -DNDEBUG -g -std=c++17 -I<src> -I<build> pointcloud_drc.cpp \
//       -L<build> -ldraco -o pointcloud_drc_cpp
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "draco/compression/decode.h"
#include "draco/compression/encode.h"
#include "draco/point_cloud/point_cloud.h"

namespace {

// The same deterministic cloud examples/pointcloud_drc.rs builds.
std::unique_ptr<draco::PointCloud> Build(int num_points) {
    auto pc = std::unique_ptr<draco::PointCloud>(new draco::PointCloud());
    pc->set_num_points(num_points);

    draco::GeometryAttribute pos;
    pos.Init(draco::GeometryAttribute::POSITION, nullptr, 3, draco::DT_FLOAT32,
             false, sizeof(float) * 3, 0);
    const int pos_id = pc->AddAttribute(pos, true, num_points);

    draco::GeometryAttribute col;
    col.Init(draco::GeometryAttribute::COLOR, nullptr, 3, draco::DT_UINT8, true,
             sizeof(uint8_t) * 3, 0);
    const int col_id = pc->AddAttribute(col, true, num_points);

    for (int i = 0; i < num_points; ++i) {
        const float xyz[3] = {
            static_cast<float>((i * 17) % 997) * 0.125f,
            static_cast<float>((i * 31) % 991) * 0.25f,
            static_cast<float>((i * 47) % 983) * 0.5f,
        };
        pc->attribute(pos_id)->SetAttributeValue(
            draco::AttributeValueIndex(i), xyz);
        const uint8_t rgb[3] = {
            static_cast<uint8_t>(i & 255),
            static_cast<uint8_t>((i * 3) & 255),
            static_cast<uint8_t>((i * 7) & 255),
        };
        pc->attribute(col_id)->SetAttributeValue(
            draco::AttributeValueIndex(i), rgb);
    }
    return pc;
}

void Configure(draco::Encoder* encoder, bool kdtree) {
    encoder->SetEncodingMethod(kdtree ? draco::POINT_CLOUD_KD_TREE_ENCODING
                                      : draco::POINT_CLOUD_SEQUENTIAL_ENCODING);
    encoder->SetSpeedOptions(5, 5);
    encoder->SetAttributeQuantization(draco::GeometryAttribute::POSITION, 10);
}

size_t Encode(const draco::PointCloud& pc, bool kdtree) {
    draco::Encoder encoder;
    Configure(&encoder, kdtree);
    draco::EncoderBuffer buffer;
    const auto status = encoder.EncodePointCloudToBuffer(pc, &buffer);
    if (!status.ok()) {
        std::fprintf(stderr, "encode failed: %s\n", status.error_msg());
        std::exit(1);
    }
    return buffer.size();
}

}  // namespace

int main(int argc, char** argv) {
    if (argc < 4) {
        std::fprintf(stderr,
                     "usage: pointcloud_drc <encode|decode> "
                     "<sequential|kdtree> <points> [iters]\n");
        return 2;
    }
    const std::string operation = argv[1];
    const std::string method = argv[2];
    const int num_points = std::atoi(argv[3]);
    const int iters = argc > 4 ? std::atoi(argv[4]) : 1;
    const bool kdtree = method == "kdtree";

    auto pc = Build(num_points);

    // Encoded once outside the loop either way, for the same reason the Rust
    // driver does it: a decode measurement must not be charged an encode.
    draco::Encoder encoder;
    Configure(&encoder, kdtree);
    draco::EncoderBuffer seed;
    const auto status = encoder.EncodePointCloudToBuffer(*pc, &seed);
    if (!status.ok()) {
        std::fprintf(stderr, "seed encode failed: %s\n", status.error_msg());
        return 1;
    }
    const std::vector<char> encoded(seed.data(), seed.data() + seed.size());

    size_t bytes = encoded.size();
    int64_t points = 0;
    if (operation == "encode") {
        for (int i = 0; i < iters; ++i) {
            bytes = Encode(*pc, kdtree);
        }
    } else if (operation == "decode") {
        for (int i = 0; i < iters; ++i) {
            draco::DecoderBuffer buffer;
            buffer.Init(encoded.data(), encoded.size());
            draco::Decoder decoder;
            auto result = decoder.DecodePointCloudFromBuffer(&buffer);
            if (!result.ok()) {
                std::fprintf(stderr, "decode failed: %s\n",
                             result.status().error_msg());
                return 1;
            }
            points = result.value()->num_points();
        }
    } else {
        std::fprintf(stderr, "unknown operation %s\n", operation.c_str());
        return 2;
    }

    std::fprintf(stderr,
                 "cpp %s %s: %d points -> %zu bytes (decoded %lld) x%d\n",
                 operation.c_str(), method.c_str(), num_points, bytes,
                 (long long)points, iters);
    return 0;
}
