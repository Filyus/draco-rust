# Geometric normal overflow fixtures

Streams whose geometric normal prediction
(`MeshPredictionSchemeGeometricNormalPredictorArea::ComputePredictedValue`)
takes one of upstream's overflow paths. Each fixture comes with
`<stem>.normals_golden.bin`: every point's normal as decoded by the Draco 1.5.7
CLI, written as little-endian `f32` bit patterns, one triplet per point, sorted.
The tests are in `crates/draco-core/tests/drc_edge_cases_test.rs`.

| Fixture | Encoder | Command options | Reaches |
| --- | --- | --- | --- |
| `icosphere.one_triangle_int32.1.0.0.drc` | Draco 1.0.0 | `-cl 10 -qp 20 -qn 10` | `ONE_TRIANGLE` with an abs sum past 2^31, which upstream truncates to `int32` before comparing it with 2^29 and dividing by its quotient |
| `ellipsoid.one_triangle_count.1.0.0.drc` | Draco 1.0.0 | `-cl 10 -qp 16 -qn 30` | `ONE_TRIANGLE` under 2^31, where only the face-count multiple matters: upstream adds the corner's own triangle once per face around the vertex, so the quotient differs from one taken over a single triangle |
| `wound_fan.saturated_abs_sum.2.2.drc` | Draco 1.5.7 | `-cl 10 -qn 10` | `TRIANGLE_AREA` with an abs sum past `i64::MAX`, where `VectorD::AbsSum` saturates rather than wraps |

Draco 1.0.0 (bitstream 2.0) is the only release that writes `ONE_TRIANGLE`:
its encoder writes mode 0 unconditionally, 1.1.0 (2.1) writes the predictor's
`TRIANGLE_AREA`, and 2.2 dropped the field. Modern decoders keep the `int32`
truncation and the per-face repetition so they decode what 1.0.0 encoded.
`VectorD::AbsSum` did not saturate until after 1.3.0; at the scales of the two
1.0.0 fixtures the 64-bit sums stay in range, so the two agree on them.

`generate.py` rebuilds all six files from fixed seeds:

```sh
python -I generate.py <1.0.0 draco_encoder> <1.5.7 draco_encoder> <1.5.7 draco_decoder> <scratch dir>
```

It also prints how many corners a model of the predictor expects to differ
from the old Rust behaviour. That count is only a guide, and at least once it
predicted no `int32` cases where the real quantization had some. What
establishes that a fixture pins its case is a red check: with only the change
it pins reverted, its test has to fail and the other tests have to pass. The
ellipsoid's squash factor, 0.5375, was picked that way: there, putting back a
single-triangle `ONE_TRIANGLE` fails the test, while dropping the `int32`
truncation does not.

- The 1.0.0 meshes are subdivided icospheres with float positions. At 20
  position bits every corner's sum passes 2^31. The ellipsoid is a regular
  icosphere turned off the axes and squashed along y and z, so its corners
  share one scale near the band where the multiple decides the quotient. That
  band moves the prediction by less than one unit of a 2^29-scale vector, so
  only a 30-bit octahedral grid (`-qn 30`) resolves it.
- The wound fan is a closed double cone with `int32` positions, read from a
  PLY with `int` properties. Its ring of twelve vertices winds four times
  round a triangle on the corners of the box, so both apexes sum to an abs sum
  of about `1.33 * 2^63`. The triangle is skewed off the diagonal so the three
  components differ: on the diagonal the wrapped sum and the saturated sum
  give the same canonical direction. Positions stay within +-2^29 because the
  1.5.7 encoder crashes on integer positions that span 2^31.
