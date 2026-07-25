//! Read an FBX document and write it back out, to open the result in another
//! importer.
//!
//! ```text
//! cargo run --example fbx_rewrite -- in.fbx out.fbx
//! ```
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
//! bpy.ops.import_scene.fbx(filepath=r'out.fbx')
//! for o in bpy.data.objects: print(o.type, o.name, o.data)
//! "
//! ```

use draco_io::FbxScene;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        return Err("usage: fbx_rewrite <in.fbx> <out.fbx>".into());
    };
    let scene = FbxScene::from_bytes(&std::fs::read(input)?)?;
    let bytes = scene.to_bytes()?;
    std::fs::write(output, &bytes)?;
    println!("{input} -> {output}: {} bytes", bytes.len());
    Ok(())
}
