# InstancedQuads.gltf

Authored for this repository rather than copied from anywhere, because no
asset in `KhronosSampleModels` uses `EXT_mesh_gpu_instancing` and the
extension needed a file to be read from. Released under **CC0 1.0
Universal**, like the sample assets it sits beside.

One unit quad, drawn four times: the copies stand 1.5 apart, grow from 0.4
to full size, and each is turned an eighth of a turn further than the last.
The spacing is deliberate — a unit quad turned 45 degrees is 1.41 across, so
anything closer would let two copies touch, and a gate that looks for four
separate bands in the frame would see three.

Everything is embedded as a data URI, so the file stands alone.
