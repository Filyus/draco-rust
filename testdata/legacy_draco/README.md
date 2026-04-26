# Legacy Draco compatibility fixtures

These fixtures are small smoke-test assets generated with legacy C++ Draco
release tools. They intentionally cover the Rust decoder policy floor of Draco
1.0.0+ without adding a full version matrix.

The repository does not assume any local legacy tool directory. Optional tests
that compare against legacy decoders can be enabled by setting:

- `DRACO_LEGACY_DECODER_1_0_0`
- `DRACO_LEGACY_DECODER_1_1_0`

| Fixture | Source | Encoder | Command options | Expected header |
| --- | --- | --- | --- | --- |
| `cube_att.mesh_seq.1.0.0.drc` | `../cube_att.obj` | Draco 1.0.0 | `-cl 0` | `v2.0 mesh method=0` |
| `cube_att.mesh_eb.1.0.0.drc` | `../cube_att.obj` | Draco 1.0.0 | `-cl 10` | `v2.0 mesh method=1` |
| `cube_att.mesh_seq.1.1.0.drc` | `../cube_att.obj` | Draco 1.1.0 | `-cl 0` | `v2.1 mesh method=0` |
| `cube_att.mesh_eb.1.1.0.drc` | `../cube_att.obj` | Draco 1.1.0 | `-cl 10` | `v2.1 mesh method=1` |
| `point_cloud_pos_norm.seq.1.0.0.drc` | `../point_cloud_test_pos_norm.ply` | Draco 1.0.0 | `-point_cloud -cl 0` | `v2.0 point_cloud method=0` |
| `point_cloud_pos_norm.seq.1.1.0.drc` | `../point_cloud_test_pos_norm.ply` | Draco 1.1.0 | `-point_cloud -cl 0` | `v2.1 point_cloud method=0` |
