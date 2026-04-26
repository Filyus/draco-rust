use crate::prediction_scheme::PredictionSchemeTransformType;
use std::marker::PhantomData;

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::prediction_scheme::PredictionSchemeDecodingTransform;

#[cfg(feature = "encoder")]
use crate::prediction_scheme::PredictionSchemeEncodingTransform;

#[cfg(feature = "encoder")]
pub struct PredictionSchemeWrapEncodingTransform<DataType> {
    num_components: usize,
    min_value: DataType,
    max_value: DataType,
    max_dif: DataType,
    min_correction: DataType,
    max_correction: DataType,
    _marker: PhantomData<DataType>,
}

#[cfg(feature = "encoder")]
impl<DataType> Default for PredictionSchemeWrapEncodingTransform<DataType>
where
    DataType: Copy + Ord + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "encoder")]
impl<DataType> PredictionSchemeWrapEncodingTransform<DataType>
where
    DataType: Copy + Ord + Default,
{
    pub fn new() -> Self {
        Self {
            num_components: 0,
            min_value: DataType::default(),
            max_value: DataType::default(),
            max_dif: DataType::default(),
            min_correction: DataType::default(),
            max_correction: DataType::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "encoder")]
impl PredictionSchemeEncodingTransform<i32, i32> for PredictionSchemeWrapEncodingTransform<i32> {
    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Wrap
    }

    fn init(&mut self, orig_data: &[i32], size: usize, num_components: usize) {
        self.num_components = num_components;

        if size == 0 {
            return;
        }

        let mut min_val = orig_data[0];
        let mut max_val = orig_data[0];

        for i in 1..size {
            let val = orig_data[i];
            if val < min_val {
                min_val = val;
            }
            if val > max_val {
                max_val = val;
            }
        }

        self.min_value = min_val;
        self.max_value = max_val;

        // InitCorrectionBounds
        let dif = (max_val as i64) - (min_val as i64);

        self.max_dif = (1 + dif) as i32;
        self.max_correction = self.max_dif / 2;
        self.min_correction = -self.max_correction;
        if (self.max_dif & 1) == 0 {
            self.max_correction -= 1;
        }
    }

    fn compute_correction(
        &self,
        original_vals: &[i32],
        predicted_vals: &[i32],
        out_corr_vals: &mut [i32],
    ) {
        for i in 0..self.num_components {
            // Clamp predicted value
            let mut pred = predicted_vals[i];
            if pred > self.max_value {
                pred = self.max_value;
            } else if pred < self.min_value {
                pred = self.min_value;
            }

            let mut corr_val = original_vals[i].wrapping_sub(pred);

            // Wrap around
            if corr_val < self.min_correction {
                corr_val = corr_val.wrapping_add(self.max_dif);
            } else if corr_val > self.max_correction {
                corr_val = corr_val.wrapping_sub(self.max_dif);
            }

            out_corr_vals[i] = corr_val;
        }
    }

    fn encode_transform_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        buffer.extend_from_slice(&self.min_value.to_le_bytes());
        buffer.extend_from_slice(&self.max_value.to_le_bytes());
        true
    }
}

#[cfg(feature = "decoder")]
pub struct PredictionSchemeWrapDecodingTransform<DataType> {
    num_components: usize,
    min_value: DataType,
    max_value: DataType,
    max_dif: DataType,
    _marker: PhantomData<DataType>,
}

#[cfg(feature = "decoder")]
impl<DataType> Default for PredictionSchemeWrapDecodingTransform<DataType>
where
    DataType: Copy + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "decoder")]
impl<DataType> PredictionSchemeWrapDecodingTransform<DataType>
where
    DataType: Copy + Default,
{
    pub fn new() -> Self {
        Self {
            num_components: 0,
            min_value: DataType::default(),
            max_value: DataType::default(),
            max_dif: DataType::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl PredictionSchemeDecodingTransform<i32, i32> for PredictionSchemeWrapDecodingTransform<i32> {
    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Wrap
    }

    #[inline]
    fn init(&mut self, num_components: usize) {
        self.num_components = num_components;
    }

    #[inline(always)]
    fn compute_original_value(
        &self,
        predicted_vals: &[i32],
        corr_vals: &[i32],
        out_original_vals: &mut [i32],
    ) {
        for i in 0..self.num_components {
            let mut pred = predicted_vals[i];
            if pred < self.min_value {
                pred = self.min_value;
            } else if pred > self.max_value {
                pred = self.max_value;
            }

            let mut val = pred.wrapping_add(corr_vals[i]);

            if val < self.min_value {
                val = val.wrapping_add(self.max_dif);
            } else if val > self.max_value {
                val = val.wrapping_sub(self.max_dif);
            }

            out_original_vals[i] = val;
        }
    }

    fn decode_transform_data(&mut self, buffer: &mut DecoderBuffer) -> bool {
        if let Ok(min_val) = buffer.decode::<i32>() {
            self.min_value = min_val;
        } else {
            return false;
        }
        if let Ok(max_val) = buffer.decode::<i32>() {
            self.max_value = max_val;
        } else {
            return false;
        }

        let dif = (self.max_value as i64) - (self.min_value as i64);
        self.max_dif = (1 + dif) as i32;

        true
    }
}
