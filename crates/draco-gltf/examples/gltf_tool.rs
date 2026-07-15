//! Small deterministic interoperability helper used by CI and by humans.

use std::io;
use std::path::PathBuf;

use draco_gltf::{GltfCompressionOptions, Import, OutputFormat};

fn decoded_draco_stats(import: &Import) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let mut primitives = 0usize;
    let mut faces = 0usize;
    for (_, primitive) in import.draco_primitives() {
        let mesh = import.decode_primitive(&primitive)?;
        if mesh.num_faces() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Draco primitive decoded to zero faces",
            )
            .into());
        }
        primitives = primitives.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "decoded primitive count overflow",
            )
        })?;
        faces = faces.checked_add(mesh.num_faces()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "decoded face count overflow")
        })?;
    }
    if primitives == 0 || faces == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "document contains no decodable Draco triangle faces",
        )
        .into());
    }
    Ok((primitives, faces))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: gltf_tool <input.gltf|input.glb> [output.glb]",
        )
    })?);
    let output = args.next().map(PathBuf::from);
    if args.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many arguments").into());
    }

    let import = draco_gltf::import(&input)?;
    if let Some(output) = output {
        let options = GltfCompressionOptions {
            output_format: OutputFormat::Glb,
            ..GltfCompressionOptions::default()
        };
        let compressed = import.compress_with_options(&options)?;
        let verified = draco_gltf::import_slice(&compressed.data, None)?;
        let (primitives, faces) = decoded_draco_stats(&verified)?;
        std::fs::write(output, &compressed.data)?;
        println!("compression_report={:?}", compressed.report);
        println!("decoded_draco_primitives={primitives} decoded_faces={faces}");
    } else {
        let (primitives, faces) = decoded_draco_stats(&import)?;
        println!("decoded_draco_primitives={primitives} decoded_faces={faces}");
    }
    Ok(())
}
