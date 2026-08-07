use draco_core::attribute_quantization_transform::AttributeQuantizationTransform;
use draco_core::attribute_transform::AttributeTransform;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::prediction_scheme::EntryToPointIdMap;

fn create_complex_mesh_pos_attr() -> PointAttribute {
    let grid_size = 50;
    let num_points = grid_size * grid_size;
    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points as usize,
    );
    for y in 0..grid_size {
        for x in 0..grid_size {
            let index = y * grid_size + x;
            let px = x as f32;
            let py = y as f32;
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;
            let offset = index as usize * 3 * 4;
            pos_attr
                .buffer_mut()
                .update(&px.to_le_bytes(), Some(offset));
            pos_attr
                .buffer_mut()
                .update(&py.to_le_bytes(), Some(offset + 4));
            pos_attr
                .buffer_mut()
                .update(&pz.to_le_bytes(), Some(offset + 8));
        }
    }
    pos_attr
}

#[test]
fn dump_rust_qt() {
    // Arrange: set debug env vars so the transform appends to a file.
    std::env::set_var("DRACO_DEBUG_CMP_CPP", "1");
    std::env::set_var("DRACO_DEBUG_CMP_CPP_FILE", "artifacts/rust_qt_dump.txt");

    let pos_attr = create_complex_mesh_pos_attr();
    let mut transform = AttributeQuantizationTransform::new();
    transform
        .compute_parameters(&pos_attr, 10)
        .expect("compute_parameters failed");

    // Optionally dump for an explicit list of original point ids via env var
    let mut point_ids: Vec<PointIndex> = Vec::new();
    if let Ok(list) = std::env::var("DRACO_DEBUG_CMP_PTIDS") {
        for part in list.split(',') {
            if let Ok(n) = part.trim().parse::<u32>() {
                point_ids.push(PointIndex(n));
            }
        }
    }

    let mut target_attr = PointAttribute::new();
    let transformed = if point_ids.is_empty() {
        transform.transform_attribute(&pos_attr, EntryToPointIdMap::identity(0), &mut target_attr)
    } else {
        transform.transform_attribute(
            &pos_attr,
            EntryToPointIdMap::from_point_indices(&point_ids),
            &mut target_attr,
        )
    };
    transformed.expect("transform_attribute failed");

    // Check that the file was written (best-effort). The test passes if no panic occurs.
    // This file will be inspected manually by the debug workflow.
}
