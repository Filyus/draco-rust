# MirroredQuads.gltf

Authored for this repository rather than copied from anywhere, because the
sample assets that mirror a node do it inside a character where the effect is
hard to see. Released under **CC0 1.0 Universal**, like the sample assets it
sits beside.

One unit quad, wound counter-clockwise as seen from `+Z` and drawn by two
nodes: one standing at `x = 1`, one at `x = -1` under a scale of `-1` on X.
The mirror reverses the winding of every triangle under it, so a renderer that
does not reverse its front-face rule with it sees the mirrored copy's back and
culls it — the material is single-sided, which is what makes the miss visible
rather than merely mis-lit. Both copies face the camera, so a correct frame has
two bands and a wrong one has the left band missing.

The material emits green so coverage can be read straight off the frame without
depending on how the viewer lights a surface.

Everything is embedded as a data URI, so the file stands alone.
