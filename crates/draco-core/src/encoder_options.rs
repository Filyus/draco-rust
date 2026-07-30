use std::collections::HashMap;

/// Integer option bag used to configure Draco encoding.
///
/// Options mirror the C++ Draco encoder style: global options apply to the
/// whole geometry, while attribute options override a value for one attribute
/// id and fall back to the global value when unset.
///
/// Common keys include `quantization_bits` (per-attribute precision),
/// `encoding_speed`/`decoding_speed`, `encoding_method`, and
/// `prediction_scheme`. Keys without an explicit setter are read and written
/// with [`get_global_int`](EncoderOptions::get_global_int) /
/// [`set_global_int`](EncoderOptions::set_global_int).
///
/// # Examples
///
/// ```
/// use draco_core::EncoderOptions;
///
/// let mut options = EncoderOptions::new();
/// options.set_global_int("quantization_bits", 14); // default for all attributes
/// options.set_attribute_int(0, "quantization_bits", 10); // override attribute 0
///
/// assert_eq!(options.get_attribute_int(0, "quantization_bits", 0), 10);
/// // Attribute 1 has no override, so it falls back to the global value.
/// assert_eq!(options.get_attribute_int(1, "quantization_bits", 0), 14);
/// ```
#[derive(Debug, Clone, Default)]
pub struct EncoderOptions {
    global_options: HashMap<String, i32>,
    attribute_options: HashMap<i32, HashMap<String, i32>>,
}

impl EncoderOptions {
    /// Creates options with Draco-compatible defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the configured encoding speed, defaulting to 5.
    pub fn get_encoding_speed(&self) -> i32 {
        self.get_global_int("encoding_speed", 5)
    }

    /// Returns the configured decoding speed target, defaulting to 5.
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

    /// Returns the forced prediction scheme, or -1 for the encoder default.
    pub fn get_prediction_scheme(&self) -> i32 {
        self.get_global_int("prediction_scheme", -1)
    }

    /// Returns the prediction scheme forced for one attribute, falling back to
    /// the global setting and then to -1 for the encoder default.
    ///
    /// Upstream reads this per attribute (`GetPredictionMethodFromOptions`), and
    /// `GetAttributeInt` itself falls back to the global option, so a value set
    /// either way is honoured.
    pub fn get_attribute_prediction_scheme(&self, att_id: i32) -> i32 {
        self.get_attribute_int(att_id, "prediction_scheme", -1)
    }

    /// Forces a prediction scheme by numeric Draco method id.
    pub fn set_prediction_scheme(&mut self, value: i32) {
        self.set_global_int("prediction_scheme", value);
    }

    /// Returns the forced encoding method, if one was set.
    pub fn get_encoding_method(&self) -> Option<i32> {
        self.global_options.get("encoding_method").cloned()
    }

    /// Forces an encoding method by numeric Draco method id.
    pub fn set_encoding_method(&mut self, value: i32) {
        self.set_global_int("encoding_method", value);
    }

    /// Sets the target Draco bitstream version.
    pub fn set_version(&mut self, major: u8, minor: u8) {
        self.set_global_int("version_major", major as i32);
        self.set_global_int("version_minor", minor as i32);
    }

    /// Returns the target Draco bitstream version, or `(0, 0)` for default.
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

    /// Sets a global integer option.
    pub fn set_global_int(&mut self, key: &str, value: i32) {
        self.global_options.insert(key.to_string(), value);
    }

    /// Returns a global integer option or the supplied default.
    pub fn get_global_int(&self, key: &str, default_val: i32) -> i32 {
        *self.global_options.get(key).unwrap_or(&default_val)
    }

    /// Sets an integer option for one attribute id.
    pub fn set_attribute_int(&mut self, att_id: i32, key: &str, value: i32) {
        self.attribute_options
            .entry(att_id)
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Returns an attribute integer option, falling back to the global value.
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
