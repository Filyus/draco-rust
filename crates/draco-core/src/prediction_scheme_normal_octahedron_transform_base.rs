use crate::normal_compression_utils::OctahedronToolBox;
use crate::prediction_scheme::PredictionSchemeTransformType;

pub struct PredictionSchemeNormalOctahedronTransformBase {
    max_quantized_value: i32,
    center_value: i32,
    octahedron_tool_box: OctahedronToolBox,
}

impl PredictionSchemeNormalOctahedronTransformBase {
    pub fn new(max_quantized_value: i32) -> Self {
        let mut octahedron_tool_box = OctahedronToolBox::new();
        // Keep defaults for uninitialized base. Actual values are set via
        // set_max_quantized_value() during DecodeTransformData().
        if max_quantized_value > 0 {
            let plus_one = (max_quantized_value as u32).wrapping_add(1);
            if plus_one.is_power_of_two() {
                let quantization_bits = plus_one.trailing_zeros() as i32;
                let _ = octahedron_tool_box.set_quantization_bits(quantization_bits);
            }
        }

        Self {
            max_quantized_value,
            center_value: max_quantized_value / 2,
            octahedron_tool_box,
        }
    }

    pub fn set_max_quantized_value(&mut self, max_quantized_value: i32) -> bool {
        // Draco expects max_quantized_value to be of form 2^q - 1.
        // (See C++ PredictionSchemeNormalOctahedron*TransformBase.)
        if max_quantized_value <= 0 {
            return false;
        }
        if (max_quantized_value & 1) == 0 {
            return false;
        }

        let plus_one = (max_quantized_value as u32).wrapping_add(1);
        if !plus_one.is_power_of_two() {
            return false;
        }

        let quantization_bits = plus_one.trailing_zeros() as i32;
        if !self
            .octahedron_tool_box
            .set_quantization_bits(quantization_bits)
        {
            return false;
        }

        self.max_quantized_value = max_quantized_value;
        self.center_value = max_quantized_value / 2;
        true
    }

    pub fn get_type() -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::NormalOctahedron
    }

    pub fn are_corrections_positive(&self) -> bool {
        true
    }

    pub fn max_quantized_value(&self) -> i32 {
        self.max_quantized_value
    }

    pub fn center_value(&self) -> i32 {
        self.center_value
    }

    pub fn quantization_bits(&self) -> i32 {
        self.octahedron_tool_box.quantization_bits()
    }

    pub fn is_in_diamond(&self, s: i32, t: i32) -> bool {
        self.octahedron_tool_box.is_in_diamond(s, t)
    }

    pub fn invert_diamond(&self, s: &mut i32, t: &mut i32) {
        self.octahedron_tool_box.invert_diamond(s, t)
    }

    pub fn mod_max(&self, x: i32) -> i32 {
        self.octahedron_tool_box.mod_max(x)
    }

    pub fn make_positive(&self, x: i32) -> i32 {
        self.octahedron_tool_box.make_positive(x)
    }
}
