# Web converter changelog

The converter and its WASM wrappers are not published to crates.io: every
wrapper ships as a zipped release asset on whatever crate release builds them
(stamped with that crate's version, since the set travels together), and the
converter itself deploys to GitHub Pages from `main`. This file records what
has changed between those shippings; when a release is prepared, the
`Unreleased` section folds into that release's notes and starts empty.

## Unreleased

- `ktx2-wasm` names the single- and two-channel transcode targets the crate
  gained — `bc4`, `bc5`, `eac_r11`, `eac_rg11` — so a viewer can take a
  normal map or a channel mask in the format drawn for it.
- The viewer's format choice ranks a texture every material samples through
  `normalTexture` as a normal map and takes BC5 on the desktop family and
  EAC RG11 on the mobile one, with the color ranking as the fallback.
