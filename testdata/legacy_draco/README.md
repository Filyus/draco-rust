# Legacy Draco compatibility fixtures

These fixtures are small compatibility assets generated with legacy C++ Draco
release tools. They cover the Rust decoder policy floor of Draco 1.0.0+ plus
targeted pre-1.0 compatibility paths that have historically differed from
modern Draco output.

The repository does not assume any local legacy tool directory. Optional tests
that compare against legacy decoders can be enabled by setting:

- `DRACO_LEGACY_DECODER_0_9_1`
- `DRACO_LEGACY_DECODER_0_10_0`
- `DRACO_LEGACY_DECODER_1_0_0`
- `DRACO_LEGACY_DECODER_1_1_0`
- `DRACO_LEGACY_DECODER_1_3_0`

| Fixture | Source | Encoder | Command options | Expected header |
| --- | --- | --- | --- | --- |
| `cube_att.mesh_seq.1.0.0.drc` | `../cube_att.obj` | Draco 1.0.0 | `-cl 0` | `v2.0 mesh method=0` |
| `cube_att.mesh_eb.1.0.0.drc` | `../cube_att.obj` | Draco 1.0.0 | `-cl 10` | `v2.0 mesh method=1` |
| `cube_att.mesh_seq.1.1.0.drc` | `../cube_att.obj` | Draco 1.1.0 | `-cl 0` | `v2.1 mesh method=0` |
| `cube_att.mesh_eb.1.1.0.drc` | `../cube_att.obj` | Draco 1.1.0 | `-cl 10` | `v2.1 mesh method=1` |
| `point_cloud_pos_norm.seq.1.0.0.drc` | `../point_cloud_test_pos_norm.ply` | Draco 1.0.0 | `-point_cloud -cl 0` | `v2.0 point_cloud method=0` |
| `point_cloud_pos_norm.seq.1.1.0.drc` | `../point_cloud_test_pos_norm.ply` | Draco 1.1.0 | `-point_cloud -cl 0` | `v2.1 point_cloud method=0` |
| `point_cloud_pos_norm.kd.1.3.0.drc` | `../point_cloud_test_pos_norm.ply` | Draco 1.3.0 | `-point_cloud -cl 10` | `v2.3 point_cloud method=1`. 1.3.0 is the first release with the KD-tree point-cloud encoder; confirms the point-cloud bitstream version comes from the geometry type alone, not the method -- sequential point clouds at this same release still claim v2.3, not v1.3. |
| `point_cloud_pos_norm_color.seq.1.0.0.drc` | `../point_cloud_test_pos_norm_color.ply` | Draco 1.0.0 | `-point_cloud -cl 0` | `v2.0 point_cloud method=0, RGBA color` |
| `point_cloud_pos_norm_color.seq.1.1.0.drc` | `../point_cloud_test_pos_norm_color.ply` | Draco 1.1.0 | `-point_cloud -cl 0` | `v2.1 point_cloud method=0, RGBA color` |
| `point_cloud_pos_norm_color.kd.1.3.0.drc` | `../point_cloud_test_pos_norm_color.ply` | Draco 1.3.0 | `-point_cloud -cl 10` | `v2.3 point_cloud method=1, RGBA color`. Colors are `Uint8` and never quantized; this is the same encoder-selection choice fixed for the current bitstream, exercised on the oldest release that has a KD-tree encoder to run it through. |
| `bun_zipper.mesh_eb_predictive.0.9.1.drc` | `../bun_zipper.ply` | Draco 0.9.1 | `-cl 10` | `v1.1 mesh method=1, predictive traversal` |
| `bun_zipper.mesh_eb_valence.0.10.0.drc` | `../bun_zipper.ply` | Draco 0.10.0 | `-cl 10` | `v1.2 mesh method=1, valence traversal` |
| `bun_zipper.mesh_eb_valence.1.0.0.drc` | `../bun_zipper.ply` | Draco 1.0.0 | `-cl 10` | `v2.0 mesh method=1, valence traversal` |
| `bun_zipper.mesh_eb_valence.1.1.0.drc` | `../bun_zipper.ply` | Draco 1.1.0 | `-cl 10` | `v2.1 mesh method=1, valence traversal` |
| `sphere_pos.mesh_eb_cmp.1.1.0.drc` | generated sphere positions | Draco 1.1.0 | `-cl 10` | `v2.1 mesh method=1, constrained multi-parallelogram` |
| `sphere_pos.mesh_eb_cmp.2.2.drc` | generated sphere positions | modern Draco | `-cl 10` | `v2.2 mesh method=1, constrained multi-parallelogram reference` |
| `sphere.mesh_eb_norm.0.9.1.drc` | generated sphere mesh with normals | Draco 0.9.1 | `-cl 10 -qn 10` | `v1.1 mesh method=1` |
| `sphere.mesh_eb_norm.1.1.0.drc` | generated sphere mesh with normals | Draco 1.1.0 | `-cl 10 -qn 10` | `v2.1 mesh method=1, canonicalized normals` |
| `sphere.mesh_eb_norm.2.2.drc` | generated sphere mesh with normals | modern Draco | `-cl 10 -qn 10` | `v2.2 mesh method=1, normal reference` |
| `test.mesh_eb_color.1.1.0.drc` | generated color mesh | Draco 1.1.0 | `-cl 10` | `v2.1 mesh method=1, color attribute` |
| `test.mesh_eb_color.2.2.drc` | generated color mesh | modern Draco | `-cl 10` | `v2.2 mesh method=1, color reference` |
| `cube_att_material.mesh_seq.1.0.0.drc` | `../cube_att_material.obj` | Draco 1.0.0 | `-cl 0 -qp 14 -qt 12 -qn 10 --metadata` | `v2.0 mesh method=0, GENERIC (material) attribute` |
| `cube_att_material.mesh_eb.1.0.0.drc` | `../cube_att_material.obj` | Draco 1.0.0 | `-cl 10 -qp 14 -qt 12 -qn 10 --metadata` | `v2.0 mesh method=1, GENERIC (material) attribute` |
| `cube_att_material.mesh_seq.1.1.0.drc` | `../cube_att_material.obj` | Draco 1.1.0 | `-cl 0 -qp 14 -qt 12 -qn 10 --metadata` | `v2.1 mesh method=0, GENERIC (material) attribute` |
| `cube_att_material.mesh_eb.1.1.0.drc` | `../cube_att_material.obj` | Draco 1.1.0 | `-cl 10 -qp 14 -qt 12 -qn 10 --metadata` | `v2.1 mesh method=1, GENERIC (material) attribute` |
| `cube_att_material.mesh_eb.2.2.drc` | `../cube_att_material.obj` | modern Draco | `-cl 10 -qp 14 -qt 12 -qn 10 --metadata` | `v2.2 mesh method=1, GENERIC reference, matching quantization`. `cube_att_material.obj` is `cube_att.obj` with its twelve faces split `usemtl matA`/`matB`, which the real OBJ reader turns into a `Uint8` GENERIC attribute -- the one attribute type the `cube_att.*` fixtures above never carry, and the only one that reaches `SequentialIntegerAttributeDecoder` unquantized. Quantization bits are pinned explicitly (rather than left at each tool's own default) because the 1.0.0/1.1.0 CLI default is `14/12/10`, not the `11/10/8` the 2.2 tool defaults to; without pinning them the sequential/EdgeBreaker legacy pair and the 2.2 reference would quantize position/texcoord/normal differently and the comparison would be tolerance-based instead of exact. Neither the 1.0.0 nor an unadorned modern OBJ writer round-trips a GENERIC attribute at all -- both need `mtllib`/`usemtl` in the source and `--metadata` at encode time so the material name reaches the stream's own attribute metadata, and 1.0.0's writer drops the attribute on export regardless, which is why this pair is checked by decoding all five fixtures with this crate's own decoder and comparing values, not by diffing against real decoder output the way the fixtures above are. |

`sphere.mesh_eb_norm.0.9.1.normals_golden.bin` stores the sorted little-endian
`f32` normal triplets decoded from `sphere.mesh_eb_norm.0.9.1.drc` by the
historical Draco 0.9.1 C++ decoder. Its octahedron-to-vector float conversion
differs from modern Draco, so the golden locks byte-exact legacy output.
