use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct EncoderOptions {
    global_options: HashMap<String, i32>,
    attribute_options: HashMap<i32, HashMap<String, i32>>,
}

impl EncoderOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_encoding_speed(&self) -> i32 {
        self.get_global_int("encoding_speed", 5)
    }

    pub fn get_decoding_speed(&self) -> i32 {
        self.get_global_int("decoding_speed", 5)
    }

    /// Returns the maximum speed for both encoding/decoding.
    /// Matches C++ ExpertEncoder::GetSpeed() behavior.
    pub fn get_speed(&self) -> i32 {
        let encoding_speed = self
            .global_options
            .get("encoding_speed")
            .copied()
            .unwrap_or(-1);
        let decoding_speed = self
            .global_options
            .get("decoding_speed")
            .copied()
            .unwrap_or(-1);
        let max_speed = encoding_speed.max(decoding_speed);
        if max_speed == -1 {
            5 // Default value
        } else {
            max_speed
        }
    }

    pub fn get_prediction_scheme(&self) -> i32 {
        self.get_global_int("prediction_scheme", -1)
    }

    pub fn set_prediction_scheme(&mut self, value: i32) {
        self.set_global_int("prediction_scheme", value);
    }

    pub fn get_encoding_method(&self) -> Option<i32> {
        self.global_options.get("encoding_method").cloned()
    }

    pub fn set_encoding_method(&mut self, value: i32) {
        self.set_global_int("encoding_method", value);
    }

    pub fn set_version(&mut self, major: u8, minor: u8) {
        self.set_global_int("version_major", major as i32);
        self.set_global_int("version_minor", minor as i32);
    }

    pub fn get_version(&self) -> (u8, u8) {
        let major = self.get_global_int("version_major", -1);
        let minor = self.get_global_int("version_minor", -1);
        if major == -1 || minor == -1 {
            // Default version depends on the encoder type and method,
            // but we'll return (0, 0) to indicate "use default".
            (0, 0)
        } else {
            (major as u8, minor as u8)
        }
    }

    pub fn set_global_int(&mut self, key: &str, value: i32) {
        self.global_options.insert(key.to_string(), value);
    }

    pub fn get_global_int(&self, key: &str, default_val: i32) -> i32 {
        *self.global_options.get(key).unwrap_or(&default_val)
    }

    pub fn set_attribute_int(&mut self, att_id: i32, key: &str, value: i32) {
        self.attribute_options
            .entry(att_id)
            .or_default()
            .insert(key.to_string(), value);
    }

    pub fn get_attribute_int(&self, att_id: i32, key: &str, default_val: i32) -> i32 {
        if let Some(opts) = self.attribute_options.get(&att_id) {
            if let Some(val) = opts.get(key) {
                return *val;
            }
        }
        // Fallback to global options if not found for attribute?
        // Draco C++ implementation does fallback to global options.
        self.get_global_int(key, default_val)
    }
}
