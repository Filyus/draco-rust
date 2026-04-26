use crate::prediction_scheme::PredictionSchemeTransformType;
use crate::prediction_scheme_normal_octahedron_transform_base::PredictionSchemeNormalOctahedronTransformBase;

pub struct PredictionSchemeNormalOctahedronCanonicalizedTransformBase {
    base: PredictionSchemeNormalOctahedronTransformBase,
}

impl PredictionSchemeNormalOctahedronCanonicalizedTransformBase {
    pub fn new(max_quantized_value: i32) -> Self {
        Self {
            base: PredictionSchemeNormalOctahedronTransformBase::new(max_quantized_value),
        }
    }

    pub fn base(&self) -> &PredictionSchemeNormalOctahedronTransformBase {
        &self.base
    }

    pub fn base_mut(&mut self) -> &mut PredictionSchemeNormalOctahedronTransformBase {
        &mut self.base
    }

    pub fn get_type() -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::NormalOctahedronCanonicalized
    }

    pub fn get_rotation_count(&self, pred: &[i32; 2]) -> i32 {
        let sign_x = pred[0];
        let sign_y = pred[1];

        if sign_x == 0 {
            if sign_y == 0 {
                0
            } else if sign_y > 0 {
                3
            } else {
                1
            }
        } else if sign_x > 0 {
            if sign_y >= 0 {
                2
            } else {
                1
            }
        } else if sign_y <= 0 {
            0
        } else {
            3
        }
    }

    pub fn rotate_point(&self, p: &[i32; 2], rotation_count: i32) -> [i32; 2] {
        match rotation_count {
            1 => [p[1], -p[0]],
            2 => [-p[0], -p[1]],
            3 => [-p[1], p[0]],
            _ => *p,
        }
    }

    pub fn rotate_point_reverse(&self, p: &[i32; 2], rotation_count: i32) -> [i32; 2] {
        // Reverse rotation is just rotating by (4 - count) % 4
        // But since we have rotate_point, we can just use it?
        // Or implement explicitly.
        // C++ doesn't have RotatePointReverse, it just calls RotatePoint with reverse count.
        // But let's keep this method for convenience if needed, or just use rotate_point.
        // Actually, let's just use rotate_point in the caller.
        // But for compatibility with existing code structure, I'll implement it.
        match rotation_count {
            1 => [-p[1], p[0]],  // Rotate by 3 (270 deg)
            2 => [-p[0], -p[1]], // Rotate by 2 (180 deg)
            3 => [p[1], -p[0]],  // Rotate by 1 (90 deg)
            _ => *p,
        }
    }

    pub fn is_in_bottom_left(&self, p: &[i32; 2]) -> bool {
        if p[0] == 0 && p[1] == 0 {
            return true;
        }
        p[0] < 0 && p[1] <= 0
    }
}
