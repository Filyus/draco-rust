# KTX2 fixtures

Where each file came from, because the answer decides what it can prove.

| file | source | codec |
|---|---|---|
| `facecap.ktx2` | extracted from three.js's `facecap.glb` | ETC1S |
| `2d_etc1s.ktx2`, `sample_etc1s.ktx2` | collected KTX2 samples | ETC1S |
| `2d_uastc.ktx2`, `sample_uastc_zstd.ktx2` | collected KTX2 samples | UASTC LDR, Zstd |
| `2d_rgba8.ktx2` | a plain `vkFormat` file, read but not transcoded | — |
| `etc1s_alpha_v250.ktx2` | written here by `basisu` **v2.50.0** | ETC1S with alpha |
| `uastc_alpha_v250.ktx2` | written here by `basisu` **v2.50.0** | UASTC LDR, Zstd |

The last two exist for a reason worth stating. Everything above them was
collected years ago, and upstream changed what its encoder writes on
2026-02-24 — "Fixing DFD of alpha channel ETC1S so it validates using
KTX-Software". This reader decides whether an ETC1S file carries alpha from the
length of its data format descriptor, 44 against 60, so a change there could
have been read wrongly with nothing to notice. It was recorded as a known limit
until a file from a newer encoder existed to check against.

It does now, and the answer is that the length is unchanged: `basisu` v2.50.0
still writes 60 bytes for ETC1S with alpha, and both files transcode
byte-identically to the reference across every target. Regenerate with:

```sh
tools/basis-cpp-oracle/vendor.sh          # clones upstream at the pinned revision
cmake -S <clone> -B <clone>/build -DCMAKE_BUILD_TYPE=Release -DBASISU_SUPPORT_OPENCL=OFF
cmake --build <clone>/build --config Release --parallel
<clone>/bin/basisu -ktx2 -mipmap -q 128 <clone>/test_files/alpha0.png \
  -output_file etc1s_alpha_v250.ktx2
<clone>/bin/basisu -ktx2 -mipmap -uastc <clone>/test_files/alpha0.png \
  -output_file uastc_alpha_v250.ktx2
```

`goldens.tsv` is not a fixture: it is what the reference transcoder produced for
every image here, one SHA-256 per line, so the ordinary test suite can check
byte-exactness without a C++ compiler. See `tools/basis-cpp-oracle`.
