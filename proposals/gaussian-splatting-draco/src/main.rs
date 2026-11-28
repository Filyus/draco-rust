use draco_gltf::{CompressionOptions, MeshIndex, OutputFormat, PrimitiveIndex};
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let mut args = std::env::args().skip(1);
    let variant = args.next();
    if matches!(
        variant.as_deref(),
        Some("read" | "clamp" | "rotation" | "signed-zero")
    ) {
        let scene = draco_gltf::import(args.next().ok_or("missing GLB path")?)?;
        let meshes = scene.document().meshes().into_iter().count();
        let primitives: Vec<_> = (0..meshes)
            .map(|index| PrimitiveIndex::new(MeshIndex(index), 0))
            .collect();
        let geometry = scene.read_primitives(&primitives)?;
        if variant.as_deref() == Some("signed-zero") {
            let splats = geometry.last().ok_or("missing splat primitive")?;
            for (semantic, component) in [
                ("KHR_gaussian_splatting:SCALE", 2),
                ("KHR_gaussian_splatting:OPACITY", 0),
            ] {
                let attribute = splats
                    .attributes()
                    .iter()
                    .find(|attribute| attribute.semantic() == semantic)
                    .ok_or("missing signed-zero attribute")?;
                assert_eq!(attribute.component_type(), draco_gltf::ComponentType::F32);
                assert!(attribute
                    .bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .skip(component)
                    .step_by(usize::from(attribute.components()))
                    .all(|bytes| *bytes == (-0.0f32).to_le_bytes()));
            }
            return Ok(());
        }
        let semantic = match variant.as_deref() {
            Some("clamp") => "KHR_gaussian_splatting:OPACITY",
            Some("rotation") => "KHR_gaussian_splatting:ROTATION",
            _ => return Ok(()),
        };
        let attribute = geometry
            .last()
            .and_then(|primitive| {
                primitive
                    .attributes()
                    .iter()
                    .find(|a| a.semantic() == semantic)
            })
            .ok_or("missing splat attribute")?;
        assert_eq!(attribute.component_type(), draco_gltf::ComponentType::F32);
        for value in attribute
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(f32::from_le_bytes)
        {
            if variant.as_deref() == Some("clamp") {
                assert_eq!(value, 1.0);
            } else {
                println!("{value:?}");
            }
        }
        return Ok(());
    }
    // `kd` and `zero` compress the generated cloud in `kd.gltf` with the
    // KD-tree encoder; `nan` stores it sequentially and lossless so a NaN
    // coefficient survives into the stream. Without a variant the sample
    // input is compressed sequentially, which is what `probe.cjs` verifies.
    let generated = variant.is_some();
    let kd_mode = matches!(variant.as_deref(), Some("kd" | "zero"));
    let input = if generated {
        dir.join("kd.gltf")
    } else {
        "testdata/GaussianSplats/tiny-splat.gltf".into()
    };
    let mut scene = draco_gltf::import(&input)?;
    let mut options = CompressionOptions::default();
    // Invalid decoded values are deliberately produced through the legacy path
    // so the conformance runner can test the experimental reader's rejection.
    let malformed = matches!(variant.as_deref(), Some("zero" | "nan" | "invalid-range"));
    options.allow_points = malformed;
    options.compress_splats = !malformed;
    if kd_mode {
        options.encoding_method = 2;
        options.quantization.position = Some(6);
        options.quantization.generic = Some(6);
    }
    if variant.as_deref() == Some("lossy") {
        options.encoding_method = 1;
        options.gaussian_splat_quantization.rotation_bits = Some(8);
    }
    scene.compress_primitive(MeshIndex(0), 0, options)?;
    std::fs::create_dir_all(&dir)?;
    let output = if generated {
        "cloud-kd.glb"
    } else {
        "cloud.glb"
    };
    std::fs::write(dir.join(output), scene.to_bytes(OutputFormat::GlbV2)?)?;
    Ok(())
}
