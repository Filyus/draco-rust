// Decodes a .drc file N times and nothing else -- the C++ half of the
// callgrind comparison, matching examples/decode_drc.rs step for step.
//
// Built directly against a Draco checkout rather than through the Rust
// bridge, because the comparison runs where valgrind does (Linux/WSL) and the
// bridge links a Windows library. See PERFORMANCE.md, the callgrind round,
// for the build line and why both sides must decode the same file.
//
//   g++ -O2 -DNDEBUG -std=c++17 -I<src> -I<build> decode_drc.cpp \
//       -L<build> -ldraco -o decode_drc
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <vector>

#include "draco/compression/decode.h"

int main(int argc, char** argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: decode_drc <file.drc> [iters]\n");
        return 2;
    }
    const int iters = argc > 2 ? std::atoi(argv[2]) : 1;

    std::ifstream file(argv[1], std::ios::binary);
    if (!file) {
        std::fprintf(stderr, "cannot open %s\n", argv[1]);
        return 2;
    }
    std::vector<char> encoded((std::istreambuf_iterator<char>(file)),
                              std::istreambuf_iterator<char>());

    int64_t points = 0, faces = 0;
    for (int i = 0; i < iters; ++i) {
        draco::DecoderBuffer buffer;
        buffer.Init(encoded.data(), encoded.size());
        draco::Decoder decoder;
        auto result = decoder.DecodeMeshFromBuffer(&buffer);
        if (!result.ok()) {
            std::fprintf(stderr, "decode failed: %s\n",
                         result.status().error_msg());
            return 1;
        }
        points = result.value()->num_points();
        faces = result.value()->num_faces();
    }
    std::fprintf(stderr, "cpp decoded %s: %lld points / %lld faces x%d\n",
                 argv[1], (long long)points, (long long)faces, iters);
    return 0;
}
