//! Read an FBX document and write it back out, to open the result in another
//! importer.
//!
//! ```text
//! cargo run --example fbx_rewrite -- in.fbx out.fbx
//! cargo run --example fbx_rewrite -- --ascii in.fbx out.fbx
//! ```
//!
//! `--ascii` writes the text container instead. ufbx reads both, so the same
//! Blender recipe below checks either one; the text file is also the only form
//! of this crate's output a person can read.
//!
//! The corpus round-trip test compares this crate's own read of both files,
//! so it agrees with itself by construction and cannot see anything only an
//! importer looks at: the class of a `Model`, `TypeFlags`, the `Definitions`
//! counts, the declared type on a `P` record. Loading both files into Blender
//! and diffing what it built is the check that can, and this is what produces
//! the second file.
//!
//! ```text
//! blender --background --python-expr "
//! import bpy; bpy.ops.wm.read_factory_settings(use_empty=True)
//! bpy.ops.wm.fbx_import(filepath=r'out.fbx')
//! for o in bpy.data.objects: print(o.type, o.name, o.data)
//! "
//! ```
//!
//! Blender 5 ships two importers and they disagree in ways that matter here.
//! `wm.fbx_import` is the C++ one built on ufbx, which is the compatibility
//! oracle this crate follows, and it resolves `Definitions` property
//! templates. `import_scene.fbx` is the legacy Python addon: it substitutes
//! its own defaults, which hides a missing property, and in Blender 5.0 it
//! raises `AttributeError: CyclesLightSettings has no attribute cast_shadow`
//! on any document containing a light -- including unmodified corpus files.

use draco_io::FbxScene;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let ascii = args.iter().any(|arg| arg == "--ascii");
    args.retain(|arg| arg != "--ascii");
    let [input, output] = args.as_slice() else {
        return Err("usage: fbx_rewrite [--ascii] <in.fbx> <out.fbx>".into());
    };
    let scene = FbxScene::from_bytes(&std::fs::read(input)?)?;
    let bytes = if ascii {
        scene.to_ascii_bytes()?
    } else {
        scene.to_bytes()?
    };
    std::fs::write(output, &bytes)?;
    println!("{input} -> {output}: {} bytes", bytes.len());
    Ok(())
}
