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

`sphere.mesh_eb_norm.0.9.1.normals_golden.bin` stores the sorted little-endian
`f32` normal triplets decoded from `sphere.mesh_eb_norm.0.9.1.drc` by the
historical Draco 0.9.1 C++ decoder. Its octahedron-to-vector float conversion
differs from modern Draco, so the golden locks byte-exact legacy output.
