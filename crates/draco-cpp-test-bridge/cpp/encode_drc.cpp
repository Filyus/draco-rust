// Encodes an .obj N times and nothing else -- the C++ half of the encode
// callgrind comparison, matching examples/encode_drc.rs step for step.
//
// Built directly against a Draco checkout rather than through the Rust
// bridge, because the comparison runs where valgrind does (Linux/WSL) and the
// bridge links a Windows library. See PERFORMANCE.md, the encode callgrind
// round, for the build line.
//
// Both sides read the same .obj, each through its own parser, so the meshes
// are only equal if the encoders agree on the output. The byte count printed
// here is that check: compare it against the Rust driver's before reading a
// single per-stage figure. The parse itself is outside the loop on both
// sides and is not part of what is being compared.
//
//   g++ -O2 -DNDEBUG -std=c++17 -I<src> -I<build> encode_drc.cpp \
//       -L<build> -ldraco -o encode_drc
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <vector>

#include "draco/core/decoder_buffer.h"

#include "draco/compression/encode.h"
#include "draco/io/obj_decoder.h"
#include "draco/mesh/mesh.h"

int main(int argc, char** argv) {
    if (argc < 3) {
        std::fprintf(stderr, "usage: encode_drc <file.obj> <speed> [iters]\n");
        return 2;
    }
    const int speed = std::atoi(argv[2]);
    const int iters = argc > 3 ? std::atoi(argv[3]) : 1;

    // Read the file here rather than through `DecodeFromFile`: a statically
    // linked Draco has no file reader registered -- the factory's registration
    // lives in a translation unit the linker drops -- so that path fails with
    // "Unable to read input file" on a perfectly good `.obj`.
    std::ifstream file(argv[1], std::ios::binary);
    if (!file) {
        std::fprintf(stderr, "cannot open %s\n", argv[1]);
        return 2;
    }
    std::vector<char> obj_text((std::istreambuf_iterator<char>(file)),
                               std::istreambuf_iterator<char>());
    draco::DecoderBuffer obj_buffer;
    obj_buffer.Init(obj_text.data(), obj_text.size());

    draco::Mesh mesh;
    draco::ObjDecoder obj_decoder;
    const auto parsed = obj_decoder.DecodeFromBuffer(&obj_buffer, &mesh);
    if (!parsed.ok()) {
        std::fprintf(stderr, "cannot parse %s: %s\n", argv[1], parsed.error_msg());
        return 2;
    }

    // The same options `common::options_for` builds: position at 11 bits, a
    // second attribute at 8, speed on both dials.
    draco::Encoder encoder;
    encoder.SetSpeedOptions(speed, speed);
    encoder.SetAttributeQuantization(draco::GeometryAttribute::POSITION, 11);
    if (mesh.num_attributes() > 1) {
        encoder.SetAttributeQuantization(draco::GeometryAttribute::NORMAL, 8);
        encoder.SetAttributeQuantization(draco::GeometryAttribute::TEX_COORD, 8);
    }

    size_t bytes = 0;
    for (int i = 0; i < iters; ++i) {
        draco::EncoderBuffer buffer;
        const auto status = encoder.EncodeMeshToBuffer(mesh, &buffer);
        if (!status.ok()) {
            std::fprintf(stderr, "encode failed: %s\n", status.error_msg());
            return 1;
        }
        bytes = buffer.size();
    }
    std::fprintf(stderr,
                 "cpp encoded %s at speed %d: %zu bytes from %d points / %d faces x%d\n",
                 argv[1], speed, bytes, (int)mesh.num_points(),
                 (int)mesh.num_faces(), iters);
    return 0;
}
