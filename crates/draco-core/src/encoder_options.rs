use std::collections::HashMap;
use std::sync::Arc;

use crate::draco_types::DataType;
use crate::geometry_attribute::GeometryAttributeType;
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;
use crate::status::{DracoError, Status};

/// Integer option bag used to configure Draco encoding.
///
/// Options mirror the C++ Draco encoder style: global options apply to the
/// whole geometry, while attribute options override a value for one attribute
/// id and fall back to the global value when unset. Integer options are the
/// historical surface and carry every decision the codec itself makes; the
/// float and float-vector options carry the explicit-quantization grid, which
/// the C++ `DracoOptions` also keeps beside its integers.
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
    inner: Arc<Inner>,
}

/// The option maps, shared until a setter needs its own copy.
///
/// The encoders clone the whole bag on every `encode` call and keep the copy
/// for the duration. A deep clone paid one allocation per map and per `String`
/// key each time -- pure bookkeeping, and on a tiny mesh a visible share of
/// the per-call floor. Behind an `Arc` the clone is a reference count, and a
/// setter on a bag that is shared at that moment copies the maps once for
/// itself (`Arc::make_mut`); a bag nobody else holds is mutated in place.
#[derive(Debug, Clone, Default)]
struct Inner {
    global_options: HashMap<String, i32>,
    attribute_options: HashMap<i32, HashMap<String, i32>>,
    global_floats: HashMap<String, f32>,
    attribute_floats: HashMap<i32, HashMap<String, f32>>,
    global_float_vectors: HashMap<String, Vec<f32>>,
    attribute_float_vectors: HashMap<i32, HashMap<String, Vec<f32>>>,
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
            .inner
            .global_options
            .get("encoding_speed")
            .copied()
            .unwrap_or(-1);
        let decoding_speed = self
            .inner
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

    /// Sets both speeds from a `draco_encoder`-style compression level.
    ///
    /// The CLI's `-cl` runs 0 (least compression) to 10 (most), the opposite
    /// sense of `encoding_speed`/`decoding_speed`, and converts with
    /// `speed = 10 - compression_level` before calling the same
    /// `SetSpeedOptions` this crate mirrors; this method does the same
    /// conversion and nothing else. `level` is not range-checked, matching
    /// the CLI, which passes an out-of-range `-cl` straight through the same
    /// subtraction rather than rejecting it.
    pub fn set_compression_level(&mut self, level: i32) {
        let speed = 10 - level;
        self.set_global_int("encoding_speed", speed);
        self.set_global_int("decoding_speed", speed);
    }

    /// Returns `10 - `[`get_speed`](Self::get_speed)`()`, the CLI's compression
    /// level for the speed this instance currently carries.
    ///
    /// A fresh `EncoderOptions` reports 5 here, not the CLI's own default of
    /// 7 — the two tools default to different speeds (5 here, 3 there), and
    /// this getter reads what is actually set rather than what the CLI would
    /// have chosen.
    pub fn get_compression_level(&self) -> i32 {
        10 - self.get_speed()
    }

    /// Sets `quantization_bits` for one attribute id.
    ///
    /// Equivalent to `ExpertEncoder::SetAttributeQuantization` and to what the
    /// CLI's `-qp`/`-qt`/`-qn`/`-qg` resolve to once they have picked an
    /// attribute id for POSITION/TEX_COORD/NORMAL/GENERIC; this method takes
    /// the id directly rather than a geometry attribute type.
    pub fn set_attribute_quantization(&mut self, att_id: i32, quantization_bits: i32) {
        self.set_attribute_int(att_id, "quantization_bits", quantization_bits);
    }

    /// Returns the `quantization_bits` set for one attribute id, or -1 if
    /// none was set for it or globally.
    pub fn get_attribute_quantization(&self, att_id: i32) -> i32 {
        self.get_attribute_int(att_id, "quantization_bits", -1)
    }

    /// Returns the forced prediction scheme, or -1 for the encoder default.
    pub fn get_prediction_scheme(&self) -> i32 {
        self.get_global_int("prediction_scheme", -1)
    }

    /// Returns the prediction scheme forced for one attribute, falling back to
    /// the global setting and then to -1 for the encoder default.
    ///
    /// Upstream reads this per attribute (`GetPredictionMethodFromOptions`), and
    /// `get_attribute_int` itself falls back to the global option, so a value
    /// set either way is honoured. Crate-internal: callers outside can already
    /// express both halves with `set_attribute_int` and `set_prediction_scheme`,
    /// and a getter with no matching per-attribute setter would be a half API.
    pub(crate) fn get_attribute_prediction_scheme(&self, att_id: i32) -> i32 {
        self.get_attribute_int(att_id, "prediction_scheme", -1)
    }

    /// Forces a prediction scheme by numeric Draco method id.
    pub fn set_prediction_scheme(&mut self, value: i32) {
        self.set_global_int("prediction_scheme", value);
    }

    /// Whether the encoder may choose a point-cloud attribute's prediction
    /// scheme by estimating the cost of each candidate.
    pub fn prediction_search(&self) -> bool {
        self.get_global_int("prediction_scheme_search", 0) != 0
    }

    /// Lets the encoder choose each attribute's prediction scheme by the
    /// estimated cost of the candidates rather than by upstream's fixed rule.
    ///
    /// Off by default, and deliberately: the automatic choice is upstream's,
    /// and this crate's output is byte-identical to C++ Draco's for the same
    /// input. Searching produces a different — smaller — stream, so it is a
    /// thing a caller asks for rather than a thing that happens to them.
    ///
    /// What it buys, and why it exists at all: the automatic choice for a
    /// point-cloud attribute is always `Difference`, and differencing costs
    /// more than it saves whenever consecutive values do not correlate.
    /// Spherical-harmonic coefficients in a Gaussian splat are the case that
    /// prompted this: predicted, they cost more bits than coded directly. The
    /// opposite case is just as real: on smoothly varying data, turning
    /// prediction off makes a file much larger. Neither is knowable from the
    /// attribute's type, so this looks at its values.
    ///
    /// **It is worth turning on only for data of that shape**, and the honest
    /// version of "that shape" is narrow. On a photogrammetry capture carrying
    /// position and colour the search finds nothing at all: both attributes
    /// are well served by differencing, and coding either directly is worse.
    /// Attributes whose values do not follow their neighbours are what this is
    /// for, and a scanned surface is the opposite of that.
    ///
    /// The cost is encode time and nothing else. The candidates are ranked by
    /// the same bit estimate the symbol coder uses to choose its own scheme,
    /// which is an entropy pass over each candidate's symbols, not a second
    /// encode, and the winner's estimate is what the coder is then handed
    /// rather than working it out again. On a cloud of many attributes the
    /// encode still takes noticeably longer. Decoding is unaffected, and every stream this can produce is one an ordinary decoder
    /// reads: the scheme is a byte the bitstream has always carried,
    /// `PREDICTION_NONE` included.
    ///
    /// An attribute with an explicit `prediction_scheme` is left alone; a
    /// caller who named a scheme has already made this choice.
    pub fn set_prediction_search(&mut self, enabled: bool) {
        self.set_global_int("prediction_scheme_search", i32::from(enabled));
    }

    /// Whether a point cloud's points may be reordered spatially before being
    /// encoded.
    pub fn spatial_point_order(&self) -> bool {
        self.get_global_int("spatial_point_order", 0) != 0
    }

    /// Lets the encoder emit a point cloud's points in a spatial order rather
    /// than in the order they were handed in.
    ///
    /// Which spatial order is the encoder's choice and not part of this
    /// option's contract: today it is a Hilbert curve, which writes fewer bytes
    /// than a Morton curve for the same encode time, and a later version may
    /// use a better one. Every such stream decodes the
    /// same way; only the order of the decoded points and the size differ.
    ///
    /// A point cloud's point order carries no meaning: no connectivity refers
    /// to it, every attribute is read through the same point index, and a
    /// decoder reconstructs whatever order the stream has. So an encoder may
    /// choose it, and choosing it spatially is what makes the difference
    /// predictor predict from a neighbour instead of from whatever the
    /// exporter happened to write next.
    ///
    /// **This is the general one of the two.** It was written for Gaussian
    /// splats, and it does more on ordinary captured geometry: a
    /// photogrammetry point cloud of coloured points gains a larger share than
    /// a splat does. Any cloud whose attributes vary through space rather than
    /// along its file order gains from it.
    ///
    /// The curve is laid over a grid as fine as the positions' own
    /// `quantization_bits`, up to 21 bits an axis. Both halves of that are
    /// measured: a coarser grid puts points the stream will distinguish into
    /// one cell, where their order is whatever the sort left them in, and a
    /// finer one sorts by differences the quantization discards.
    ///
    /// Off by default for the same reason the prediction search is: the output
    /// differs, byte for byte, from what upstream C++ Draco writes for the same
    /// input, and this crate's default is to match it.
    ///
    /// **This reorders the decoded points.** Anything outside the file that
    /// indexes into it by point number — a sidecar array, an index written by
    /// another tool — will be pointing at different points afterwards. Nothing
    /// inside a `.drc` does, which is why this is expressible at all, but a
    /// caller who has such a thing is the one who knows.
    ///
    /// **It can also make a file bigger**, and unlike the prediction search it
    /// does not check. The gain comes from attributes that vary through space;
    /// an attribute that varies along the order it was handed in — an index, a
    /// timestamp, anything written in sequence — is scrambled by the reorder
    /// and costs more afterwards: a cloud of positions plus a running integer
    /// tag grows. It is not checked because the option is a
    /// statement about the order, not about the size: a caller who wants
    /// spatial locality in the decoded cloud wants it whether or not it also
    /// happens to compress better. Whoever wants only the smaller file wants
    /// [`set_point_order_search`](Self::set_point_order_search) instead, which
    /// checks, and is allowed to trade locality for size.
    ///
    /// Applies to the sequential coder. The kd-tree coder chooses its own point
    /// order and this leaves it alone. A point cloud with no position attribute
    /// has nothing to sort by and is also left alone.
    pub fn set_spatial_point_order(&mut self, enabled: bool) {
        self.set_global_int("spatial_point_order", i32::from(enabled));
    }

    /// Whether the encoder may search for the order of a point cloud's points
    /// that makes the stream smallest.
    pub fn point_order_search(&self) -> bool {
        self.get_global_int("point_order_search", 0) != 0
    }

    /// Lets the encoder search for the order of a point cloud's points that
    /// makes the stream smallest, and leave the order alone where it finds the
    /// one it was handed already better.
    ///
    /// The sequential coder writes each attribute as the difference between a
    /// point and the one before it, so the order decides how large those
    /// differences are. A spatial order makes neighbours in space neighbours
    /// in the stream; this strings the points along a Hilbert curve and then
    /// repairs it where the other attributes disagree with the positions,
    /// which they do in a Gaussian splat, where the position is a small part
    /// of each point's bytes.
    ///
    /// Against [`set_spatial_point_order`](Self::set_spatial_point_order) at
    /// the same encoding speed it gains the most on clouds like that, and more
    /// the lower the speed. Photogrammetry scans and lidar captures gain less.
    /// The stream is an ordinary one. Only the order of the points in it
    /// differs.
    ///
    /// **It costs encode time**, several times the encode in file order, and
    /// more the lower the speed. The search runs on as many threads as
    /// [`set_threads`](Self::set_threads) allows, writing the same order on any
    /// number. **And some decode time**: in the order it finds, neighbours
    /// agree well enough that the encoder picks the more compact schemes,
    /// differences over values as they are and the raw coder over the tagged
    /// one, and those take a little longer to undo.
    ///
    /// **It can decline.** Each order it could write -- the one it was handed,
    /// the curve, and the curve repaired -- is priced by the symbol coder's own
    /// plan of what it would write for a sample of the cloud, and an order
    /// replaces another only if it undercuts it by 1%. The repair is tried on a
    /// sample of the path's blocks first, one in sixteen on a large cloud, and
    /// finished only if the trial pays. A scan or a rotating-lidar capture
    /// written along its scan lines is already in an order no curve improves
    /// on, and comes out exactly as it would without this, where a spatial
    /// order would have made it larger.
    ///
    /// **The effort follows `encoding_speed`**, the way it does elsewhere:
    /// speed 9 and 10 write the curve and nothing more, 7 and 8 count the 8
    /// dearest attribute columns over a window of 8 points, 5 and 6 (the
    /// default) 16 over 16, 3 and 4 32 over 32, and 0 to 2 the 64 dearest over
    /// 64.
    ///
    /// Off by default for the reason the prediction search is: it changes the
    /// bytes, and this crate's default is to match upstream C++ Draco.
    ///
    /// **This reorders the decoded points**, and unlike
    /// [`set_spatial_point_order`](Self::set_spatial_point_order) it makes no
    /// promise that they are spatially near one another: a point may sit
    /// beside one that looks like it rather than one that is close. If the
    /// search keeps the order it was handed and `set_spatial_point_order` is
    /// also on, the spatial order is what is written.
    ///
    /// Applies to the sequential coder, like the order option does.
    pub fn set_point_order_search(&mut self, enabled: bool) {
        self.set_global_int("point_order_search", i32::from(enabled));
    }

    /// The threads asked of this encode: 1, the default, is the calling thread
    /// alone, 0 is as many as the machine has, and no count is taken past
    /// sixteen.
    pub fn get_threads(&self) -> i32 {
        self.get_global_int("threads", 1)
    }

    /// Caps the threads an encode may use; `1`, the default, keeps everything
    /// on the calling thread and `0` takes as many as the machine has. Neither
    /// a count asked for nor the machine's is taken past sixteen.
    ///
    /// One by default because a library cannot see what its caller already
    /// runs: a server encoding on a pool of its own, or a converter working
    /// through files side by side, would have every encode multiply its
    /// threads by up to sixteen. A caller with the cores to spare, encoding a
    /// large cloud at a time, asks for them.
    ///
    /// A sequential point cloud's attributes are encoded side by side, each
    /// into a buffer of its own that is appended in order, and an attribute of
    /// a million values or more also cuts its own passes -- quantization, the
    /// gather, the prediction, the symbol plan -- into pieces; only the rANS
    /// write stays one chain. The point order search and the spatial curve run
    /// on the same threads. A large cloud of many attributes, a Gaussian splat
    /// for one, encodes several times faster on many threads. Below 2^17 values
    /// in all, counted as points times attributes, no thread is started.
    ///
    /// Only work that splits into pieces independent of one another is run on
    /// threads, and each piece's result is the same whichever thread works it,
    /// so the stream does not depend on this. The calling thread is one of
    /// them, and a thread the system refuses to start is one fewer rather than
    /// an error: the encode finishes on those that did. On WebAssembly there
    /// are no threads and the value is ignored.
    pub fn set_threads(&mut self, threads: i32) {
        self.set_global_int("threads", threads);
    }

    /// Returns the forced encoding method, if one was set.
    pub fn get_encoding_method(&self) -> Option<i32> {
        self.inner.global_options.get("encoding_method").cloned()
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
        Arc::make_mut(&mut self.inner)
            .global_options
            .insert(key.to_string(), value);
    }

    /// Returns a global integer option or the supplied default.
    pub fn get_global_int(&self, key: &str, default_val: i32) -> i32 {
        *self.inner.global_options.get(key).unwrap_or(&default_val)
    }

    /// Sets an integer option for one attribute id.
    pub fn set_attribute_int(&mut self, att_id: i32, key: &str, value: i32) {
        Arc::make_mut(&mut self.inner)
            .attribute_options
            .entry(att_id)
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Returns an attribute integer option, falling back to the global value.
    pub fn get_attribute_int(&self, att_id: i32, key: &str, default_val: i32) -> i32 {
        if let Some(opts) = self.inner.attribute_options.get(&att_id) {
            if let Some(val) = opts.get(key) {
                return *val;
            }
        }
        // Falls back to the global value, as upstream does.
        self.get_global_int(key, default_val)
    }

    /// Sets a global float option.
    pub fn set_global_float(&mut self, key: &str, value: f32) {
        Arc::make_mut(&mut self.inner)
            .global_floats
            .insert(key.to_string(), value);
    }

    /// Returns a global float option or the supplied default.
    pub fn get_global_float(&self, key: &str, default_val: f32) -> f32 {
        self.inner
            .global_floats
            .get(key)
            .copied()
            .unwrap_or(default_val)
    }

    /// Sets a global float-vector option.
    pub fn set_global_floats(&mut self, key: &str, values: &[f32]) {
        Arc::make_mut(&mut self.inner)
            .global_float_vectors
            .insert(key.to_string(), values.to_vec());
    }

    /// Returns a global float-vector option, or `None` when unset.
    pub fn get_global_floats(&self, key: &str) -> Option<&[f32]> {
        self.inner.global_float_vectors.get(key).map(Vec::as_slice)
    }

    /// Sets a float option for one attribute id.
    ///
    /// The float options exist for the explicit-quantization grid, which the
    /// C++ `DracoOptions` stores beside its integers
    /// (`SetAttributeFloat`/`SetAttributeVector`), and they fall back the same
    /// way the integer ones do: per-attribute first, then global.
    pub fn set_attribute_float(&mut self, att_id: i32, key: &str, value: f32) {
        Arc::make_mut(&mut self.inner)
            .attribute_floats
            .entry(att_id)
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Returns an attribute float option, falling back to the global value.
    pub fn get_attribute_float(&self, att_id: i32, key: &str, default_val: f32) -> f32 {
        self.get_attribute_float_opt(att_id, key)
            .unwrap_or(default_val)
    }

    /// Returns an attribute float option without a fallback, so a caller can
    /// tell "unset" from "set to the default".
    ///
    /// Crate-internal: the encoder reads `quantization_range` through
    /// [`get_attribute_float`](Self::get_attribute_float) with C++'s own
    /// default of `1.0`; only the both-options-set check for the
    /// explicit-quantization grid needs the `None`.
    pub(crate) fn get_attribute_float_opt(&self, att_id: i32, key: &str) -> Option<f32> {
        if let Some(opts) = self.inner.attribute_floats.get(&att_id) {
            if let Some(val) = opts.get(key) {
                return Some(*val);
            }
        }
        self.inner.global_floats.get(key).copied()
    }

    /// Sets a float-vector option for one attribute id.
    pub fn set_attribute_floats(&mut self, att_id: i32, key: &str, values: &[f32]) {
        Arc::make_mut(&mut self.inner)
            .attribute_float_vectors
            .entry(att_id)
            .or_default()
            .insert(key.to_string(), values.to_vec());
    }

    /// Returns an attribute float-vector option, falling back to the global
    /// value.
    pub fn get_attribute_floats(&self, att_id: i32, key: &str) -> Option<&[f32]> {
        if let Some(opts) = self.inner.attribute_float_vectors.get(&att_id) {
            if let Some(values) = opts.get(key) {
                return Some(values);
            }
        }
        self.inner.global_float_vectors.get(key).map(Vec::as_slice)
    }

    /// Sets the explicit-quantization grid for one attribute id, as
    /// `ExpertEncoder::SetAttributeExplicitQuantization` does.
    ///
    /// The three options land under the same keys C++ writes --
    /// `"quantization_bits"`, `"quantization_origin"` and
    /// `"quantization_range"` -- and reach the encoder through
    /// [`get_attribute_int`](Self::get_attribute_int),
    /// [`get_attribute_floats`](Self::get_attribute_floats) and
    /// [`get_attribute_float`](Self::get_attribute_float). Every encoder that
    /// quantizes a float attribute honours them; a grid is used only when both
    /// `quantization_origin` and `quantization_range` are set, and
    /// `compute_parameters` runs as before otherwise.
    ///
    /// `num_dims` is the grid's component count, as in C++, and the first
    /// `num_dims` values of `origin` are stored; a slice shorter than that is
    /// stored whole and refused at encode time against any attribute with more
    /// components than it holds. The bitstream does not change shape either
    /// way: `min_values` and `range` are written as always, the grid only
    /// decides where the encoder takes them from.
    pub fn set_attribute_explicit_quantization(
        &mut self,
        att_id: i32,
        quantization_bits: i32,
        num_dims: usize,
        origin: &[f32],
        range: f32,
    ) {
        self.set_attribute_int(att_id, "quantization_bits", quantization_bits);
        let origin = origin.get(..num_dims).unwrap_or(origin);
        self.set_attribute_floats(att_id, "quantization_origin", origin);
        self.set_attribute_float(att_id, "quantization_range", range);
    }

    /// Sets an explicit three-dimensional POSITION grid from a uniform step.
    ///
    /// The grid origin is the grid vertex at or below each component's minimum
    /// position. The bit count is the smallest supported count that covers the
    /// largest component span, with one bit for a single grid vertex. The range
    /// then spans `(2^bits - 1) * spacing`, so every component uses the same
    /// spacing and grid origin across separate encodes.
    ///
    /// `spacing` must be finite and positive. The selected attribute must be a
    /// non-empty, three-component, `Float32` POSITION attribute, and the grid
    /// must fit in the transform's supported 1..=30 bits. Invalid values or a
    /// malformed attribute return an error without changing these options.
    ///
    /// The bounds and grid vertices are calculated in `f64` from the exact
    /// `f32` inputs. Draco stores the origin and range as `f32`, so each is
    /// rounded by at most half an `f32` ULP; the decoded step uses that stored
    /// range.
    pub fn set_attribute_grid_quantization(
        &mut self,
        point_cloud: &PointCloud,
        att_id: i32,
        spacing: f32,
    ) -> Status {
        if !spacing.is_finite() || spacing <= 0.0 {
            return Err(DracoError::invalid_parameter(format!(
                "Grid quantization spacing must be finite and positive, got {spacing}"
            )));
        }

        let attribute = point_cloud.try_attribute(att_id)?;
        if attribute.attribute_type() != GeometryAttributeType::Position {
            return Err(DracoError::invalid_parameter(
                "Grid quantization is supported only for POSITION attributes",
            ));
        }
        if attribute.num_components() != 3 {
            return Err(DracoError::invalid_parameter(format!(
                "Grid quantization requires three-component POSITION values, got {}",
                attribute.num_components()
            )));
        }
        if attribute.data_type() != DataType::Float32 {
            return Err(DracoError::invalid_parameter(format!(
                "Grid quantization requires Float32 POSITION values, got {:?}",
                attribute.data_type()
            )));
        }
        if point_cloud.num_points() == 0 {
            return Err(DracoError::invalid_parameter(
                "Grid quantization cannot use an empty point cloud",
            ));
        }

        let stride = usize::try_from(attribute.byte_stride()).map_err(|_| {
            DracoError::invalid_parameter("Grid quantization POSITION stride is invalid")
        })?;
        if stride < 3 * std::mem::size_of::<f32>() {
            return Err(DracoError::invalid_parameter(
                "Grid quantization POSITION values are shorter than three Float32 components",
            ));
        }

        let spacing = f64::from(spacing);
        let mut min_grid = [f64::INFINITY; 3];
        let mut max_grid = [f64::NEG_INFINITY; 3];
        let data = attribute.buffer().data();
        for point in 0..point_cloud.num_points() {
            let point_index = u32::try_from(point).map_err(|_| {
                DracoError::invalid_parameter(
                    "Grid quantization point count exceeds the supported index range",
                )
            })?;
            let value_index = attribute.mapped_index(PointIndex(point_index)).0 as usize;
            if value_index >= attribute.size() {
                return Err(DracoError::invalid_parameter(format!(
                    "Grid quantization POSITION point {point} has no attribute value"
                )));
            }
            let value_start = value_index.checked_mul(stride).ok_or_else(|| {
                DracoError::invalid_parameter("Grid quantization POSITION offset overflow")
            })?;

            for component in 0..3 {
                let start = value_start
                    .checked_add(component * std::mem::size_of::<f32>())
                    .ok_or_else(|| {
                        DracoError::invalid_parameter("Grid quantization POSITION offset overflow")
                    })?;
                let end = start
                    .checked_add(std::mem::size_of::<f32>())
                    .ok_or_else(|| {
                        DracoError::invalid_parameter("Grid quantization POSITION offset overflow")
                    })?;
                let bytes = data.get(start..end).ok_or_else(|| {
                    DracoError::invalid_parameter(format!(
                        "Grid quantization POSITION value {value_index} is truncated"
                    ))
                })?;
                let value = f32::from_le_bytes(bytes.try_into().map_err(|_| {
                    DracoError::invalid_parameter("Grid quantization POSITION value is invalid")
                })?);
                if !value.is_finite() {
                    return Err(DracoError::invalid_parameter(format!(
                        "Grid quantization POSITION point {point} component {component} is not finite"
                    )));
                }

                // Inputs are f32, so widening them before division preserves
                // their exact values while avoiding an extra f32 rounding at
                // grid boundaries.
                let grid_position = f64::from(value) / spacing;
                let floor = grid_position.floor();
                let ceil = grid_position.ceil();
                min_grid[component] = min_grid[component].min(floor);
                max_grid[component] = max_grid[component].max(ceil);
            }
        }

        let max_intervals = ((1u64 << 30) - 1) as f64;
        let mut origin = [0.0f32; 3];
        let mut needed_intervals = 0u64;
        for component in 0..3 {
            let intervals = max_grid[component] - min_grid[component];
            if !intervals.is_finite() || intervals < 0.0 || intervals > max_intervals {
                return Err(DracoError::invalid_parameter(format!(
                    "Grid quantization needs {intervals} intervals on POSITION component {component}; the maximum is {}",
                    max_intervals as u64
                )));
            }
            needed_intervals = needed_intervals.max(intervals as u64);

            let origin_value = min_grid[component] * spacing;
            let narrowed_origin = origin_value as f32;
            if !origin_value.is_finite() || !narrowed_origin.is_finite() {
                return Err(DracoError::invalid_parameter(format!(
                    "Grid quantization origin on POSITION component {component} is not representable as Float32"
                )));
            }
            origin[component] = narrowed_origin;
        }

        let num_grid_values = needed_intervals + 1;
        let bits = (u64::BITS - (num_grid_values - 1).leading_zeros()).max(1) as i32;
        let max_quantized_value = (1u64 << bits) - 1;
        let range_value = max_quantized_value as f64 * spacing;
        let range = range_value as f32;
        if !range_value.is_finite() || !range.is_finite() || range <= 0.0 {
            return Err(DracoError::invalid_parameter(
                "Grid quantization range is not representable as a positive Float32",
            ));
        }

        self.set_attribute_explicit_quantization(att_id, bits, 3, &origin, range);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `draco_encoder`'s own default is `-cl 7`; this crate's is speed 5.
    /// Verified against the CLI: `EncoderOptions::new()` and
    /// `draco_encoder -cl 5 -qp 0` produce byte-identical output.
    #[test]
    fn compression_level_defaults_to_5_not_the_cli_default_of_7() {
        let options = EncoderOptions::new();
        assert_eq!(options.get_compression_level(), 5);
    }

    /// `speed = 10 - compression_level`, matching `draco_encoder.cc`'s own
    /// conversion before it calls `SetSpeedOptions`.
    #[test]
    fn set_compression_level_matches_the_cli_conversion() {
        let mut options = EncoderOptions::new();
        options.set_compression_level(7);
        assert_eq!(options.get_speed(), 3);
        assert_eq!(options.get_encoding_speed(), 3);
        assert_eq!(options.get_decoding_speed(), 3);
        assert_eq!(options.get_compression_level(), 7);
    }

    #[test]
    fn set_compression_level_round_trips_the_full_cli_range() {
        for level in 0..=10 {
            let mut options = EncoderOptions::new();
            options.set_compression_level(level);
            assert_eq!(options.get_compression_level(), level);
        }
    }

    /// A clone shares the maps, and a setter on either side afterwards
    /// changes only the side it was called on.
    #[test]
    fn a_clone_shares_the_maps_until_one_side_writes() {
        let mut options = EncoderOptions::new();
        options.set_global_int("quantization_bits", 14);
        options.set_attribute_int(1, "quantization_bits", 10);
        let mut cloned = options.clone();
        assert!(Arc::ptr_eq(&options.inner, &cloned.inner));

        cloned.set_attribute_int(1, "quantization_bits", 8);
        options.set_global_int("quantization_bits", 12);
        assert!(!Arc::ptr_eq(&options.inner, &cloned.inner));
        assert_eq!(options.get_attribute_int(1, "quantization_bits", 0), 10);
        assert_eq!(options.get_global_int("quantization_bits", 0), 12);
        assert_eq!(cloned.get_attribute_int(1, "quantization_bits", 0), 8);
        assert_eq!(cloned.get_global_int("quantization_bits", 0), 14);
    }

    #[test]
    fn attribute_quantization_defaults_to_unset() {
        let options = EncoderOptions::new();
        assert_eq!(options.get_attribute_quantization(0), -1);
    }

    #[test]
    fn set_attribute_quantization_round_trips() {
        let mut options = EncoderOptions::new();
        options.set_attribute_quantization(0, 14);
        options.set_attribute_quantization(1, 10);

        assert_eq!(options.get_attribute_quantization(0), 14);
        assert_eq!(options.get_attribute_quantization(1), 10);
        // Unset attributes fall back to the global value, same as
        // `get_attribute_int` -- there is none here, so -1.
        assert_eq!(options.get_attribute_quantization(2), -1);
    }

    #[test]
    fn float_options_fall_back_attribute_to_global() {
        let mut options = EncoderOptions::new();
        assert_eq!(
            options.get_attribute_float(0, "quantization_range", 1.0),
            1.0
        );
        assert!(options
            .get_attribute_float_opt(0, "quantization_range")
            .is_none());
        assert!(options
            .get_attribute_floats(0, "quantization_origin")
            .is_none());

        options.set_global_float("quantization_range", 4.0);
        assert_eq!(
            options.get_attribute_float(0, "quantization_range", 1.0),
            4.0
        );
        assert_eq!(
            options.get_attribute_float_opt(0, "quantization_range"),
            Some(4.0)
        );

        options.set_attribute_float(0, "quantization_range", 2.5);
        assert_eq!(
            options.get_attribute_float(0, "quantization_range", 1.0),
            2.5
        );
        // Another attribute still sees the global value.
        assert_eq!(
            options.get_attribute_float(1, "quantization_range", 1.0),
            4.0
        );
    }

    #[test]
    fn float_vectors_fall_back_attribute_to_global() {
        let mut options = EncoderOptions::new();
        options.set_global_floats("quantization_origin", &[1.0, 2.0]);
        assert_eq!(
            options.get_attribute_floats(0, "quantization_origin"),
            Some(&[1.0, 2.0][..])
        );

        options.set_attribute_floats(0, "quantization_origin", &[7.0]);
        assert_eq!(
            options.get_attribute_floats(0, "quantization_origin"),
            Some(&[7.0][..])
        );
        assert_eq!(
            options.get_attribute_floats(1, "quantization_origin"),
            Some(&[1.0, 2.0][..])
        );
    }

    /// The helper writes the three keys C++ writes, and the grid it leaves
    /// behind is what the encoders read back.
    #[test]
    fn set_attribute_explicit_quantization_writes_the_cpp_keys() {
        let mut options = EncoderOptions::new();
        options.set_attribute_explicit_quantization(2, 12, 3, &[1.0, -2.0, 5.0], 10.0);

        assert_eq!(options.get_attribute_quantization(2), 12);
        assert_eq!(
            options.get_attribute_floats(2, "quantization_origin"),
            Some(&[1.0, -2.0, 5.0][..])
        );
        assert_eq!(
            options.get_attribute_float_opt(2, "quantization_range"),
            Some(10.0)
        );
    }

    /// `num_dims` truncates the origin from the front, as C++'s
    /// `SetAttributeVector(name, num_dims, origin)` copies `num_dims` values.
    #[test]
    fn set_attribute_explicit_quantization_takes_num_dims_values() {
        let mut options = EncoderOptions::new();
        options.set_attribute_explicit_quantization(0, 10, 2, &[1.0, 2.0, 3.0], 8.0);
        assert_eq!(
            options.get_attribute_floats(0, "quantization_origin"),
            Some(&[1.0, 2.0][..])
        );
    }

    /// A clone shares the float maps the way it shares the integer ones: until
    /// one side writes.
    #[test]
    fn a_clone_shares_the_float_maps_until_one_side_writes() {
        let mut options = EncoderOptions::new();
        options.set_attribute_explicit_quantization(0, 14, 3, &[0.0, 0.0, 0.0], 1.0);
        let mut cloned = options.clone();
        assert!(Arc::ptr_eq(&options.inner, &cloned.inner));

        cloned.set_attribute_float(0, "quantization_range", 2.0);
        assert!(!Arc::ptr_eq(&options.inner, &cloned.inner));
        assert_eq!(
            options.get_attribute_float_opt(0, "quantization_range"),
            Some(1.0)
        );
        assert_eq!(
            cloned.get_attribute_float_opt(0, "quantization_range"),
            Some(2.0)
        );
    }

    #[test]
    fn set_attribute_grid_quantization_computes_origin_bits_and_range() {
        use crate::geometry_attribute::PointAttribute;

        let mut cloud = PointCloud::new();
        cloud.set_num_points(2);
        let mut positions = PointAttribute::new();
        positions.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            2,
        );
        positions.buffer_mut().update(
            &[
                0.25f32, -1.75, 2.0, // floor grid: (0, -4, 4), ceil: (1, -3, 4)
                2.5, 0.5, 2.0, // floor grid: (5, 1, 4), ceil: (5, 1, 4)
            ]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
            Some(0),
        );
        let att_id = cloud.add_attribute(positions);

        let mut options = EncoderOptions::new();
        options
            .set_attribute_grid_quantization(&cloud, att_id, 0.5)
            .expect("valid grid quantization");

        // Spans: x = 5 - 0 = 5, y = 1 - (-4) = 5, z = 4 - 4 = 0 -> 6 grid values -> 3 bits (max 7).
        assert_eq!(options.get_attribute_quantization(att_id), 3);
        assert_eq!(
            options.get_attribute_floats(att_id, "quantization_origin"),
            Some(&[0.0, -2.0, 2.0][..])
        );
        assert_eq!(
            options.get_attribute_float_opt(att_id, "quantization_range"),
            Some(3.5)
        );
    }

    #[test]
    fn set_attribute_grid_quantization_rejects_invalid_inputs_without_mutating() {
        use crate::geometry_attribute::PointAttribute;

        let mut cloud = PointCloud::new();
        cloud.set_num_points(1);
        let mut positions = PointAttribute::new();
        positions.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            1,
        );
        positions.buffer_mut().update(
            &[1.0f32, 2.0, 3.0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            Some(0),
        );
        let pos_id = cloud.add_attribute(positions);

        let mut options = EncoderOptions::new();
        assert!(options
            .set_attribute_grid_quantization(&cloud, pos_id, 0.0)
            .is_err());
        assert!(options
            .set_attribute_grid_quantization(&cloud, pos_id, -1.0)
            .is_err());
        assert!(options
            .set_attribute_grid_quantization(&cloud, pos_id, f32::NAN)
            .is_err());
        assert_eq!(options.get_attribute_quantization(pos_id), -1);
        assert_eq!(
            options.get_attribute_floats(pos_id, "quantization_origin"),
            None
        );
        assert_eq!(
            options.get_attribute_float_opt(pos_id, "quantization_range"),
            None
        );

        // Single grid vertex uses 1 bit.
        options
            .set_attribute_grid_quantization(&cloud, pos_id, 1.0)
            .expect("single vertex grid");
        assert_eq!(options.get_attribute_quantization(pos_id), 1);
        assert_eq!(
            options.get_attribute_floats(pos_id, "quantization_origin"),
            Some(&[1.0, 2.0, 3.0][..])
        );
        assert_eq!(
            options.get_attribute_float_opt(pos_id, "quantization_range"),
            Some(1.0)
        );
    }
}
