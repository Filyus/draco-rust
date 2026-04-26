#[derive(Debug, Default, Clone, Copy)]
pub struct Quantizer {
    inverse_delta: f32,
}

impl Quantizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, range: f32, max_quantized_value: i32) {
        if range > 0.0 {
            self.inverse_delta = max_quantized_value as f32 / range;
        } else {
            self.inverse_delta = 0.0;
        }
    }

    pub fn init_with_delta(&mut self, delta: f32) {
        if delta > 0.0 {
            self.inverse_delta = 1.0 / delta;
        } else {
            self.inverse_delta = 0.0;
        }
    }

    pub fn quantize_float(&self, val: f32) -> i32 {
        let val = val * self.inverse_delta;
        // Use explicit f32 literal to avoid accidental promotion to f64 and
        // to match C++'s float-floor(val + 0.5f) behavior exactly.
        (val + 0.5f32).floor() as i32
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Dequantizer {
    delta: f32,
}

impl Dequantizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, range: f32, max_quantized_value: i32) -> bool {
        if max_quantized_value > 0 {
            self.delta = range / max_quantized_value as f32;
            true
        } else {
            false
        }
    }

    pub fn init_with_delta(&mut self, delta: f32) -> bool {
        if delta >= 0.0 {
            self.delta = delta;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn dequantize_float(&self, val: i32) -> f32 {
        val as f32 * self.delta
    }
}
