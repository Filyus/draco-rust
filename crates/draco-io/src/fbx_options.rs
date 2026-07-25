//! Byte order, resource limits, and read options for the FBX binary reader.
//!
//! FBX is the only hand-rolled untrusted binary format in this crate, and its
//! node records carry file-controlled lengths that feed allocations directly.
//! The limits here bound those allocations; [`FbxReadOptions`] selects how
//! strictly the container layout is enforced.

/// Byte order of an FBX file, chosen by the header's endian marker.
///
/// The canonical Autodesk profile is little-endian (marker `0`). A non-zero
/// marker selects big-endian, matching how `ufbx` interprets the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FbxByteOrder {
    /// Canonical little-endian layout.
    Little,
    /// Big-endian layout, produced by some Maya builds.
    Big,
}

macro_rules! decode_scalar {
    ($name:ident, $ty:ty, $width:expr) => {
        /// Decodes one value in this byte order.
        #[inline]
        pub fn $name(self, bytes: [u8; $width]) -> $ty {
            match self {
                FbxByteOrder::Little => <$ty>::from_le_bytes(bytes),
                FbxByteOrder::Big => <$ty>::from_be_bytes(bytes),
            }
        }
    };
}

impl FbxByteOrder {
    decode_scalar!(u16, u16, 2);
    decode_scalar!(i16, i16, 2);
    decode_scalar!(u32, u32, 4);
    decode_scalar!(i32, i32, 4);
    decode_scalar!(u64, u64, 8);
    decode_scalar!(i64, i64, 8);
    decode_scalar!(f32, f32, 4);
    decode_scalar!(f64, f64, 8);

    /// Reverses each `N`-byte element in place when the file is big-endian.
    ///
    /// Array payloads are converted in bulk so the little-endian path keeps
    /// its original per-element decode with no added branch.
    #[inline]
    pub(crate) fn swap_elements_in_place(self, data: &mut [u8], element_size: usize) {
        if self == FbxByteOrder::Little || element_size < 2 {
            return;
        }
        for element in data.chunks_exact_mut(element_size) {
            element.reverse();
        }
    }
}

/// Bounds on what a single FBX document may allocate while decoding.
///
/// Every field is a hard ceiling: exceeding one fails the read with
/// [`std::io::ErrorKind::OutOfMemory`], which callers can distinguish from the
/// [`std::io::ErrorKind::InvalidData`] used for structural corruption.
///
/// The defaults are calibrated against real assets rather than guessed. They
/// are deliberately far above observed usage, because their job is to stop a
/// hostile header from claiming gigabytes, not to police legitimate files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FbxDecodeLimits {
    /// Largest accepted input, in bytes.
    pub max_file_bytes: u64,
    /// Deepest accepted node nesting.
    pub max_depth: u32,
    /// Largest accepted total node count.
    pub max_nodes: u64,
    /// Largest accepted property count on one node.
    pub max_properties_per_node: u64,
    /// Largest accepted `S` (string) property payload, in bytes.
    pub max_string_bytes: u64,
    /// Largest accepted `R` (raw blob) property payload, in bytes.
    ///
    /// Embedded textures arrive through `Video.Content` as `R` properties, so
    /// this is the limit most likely to matter for real scenes.
    pub max_blob_bytes: u64,
    /// Largest accepted element count in one array property.
    pub max_array_elements: u64,
    /// Largest accepted decoded size of one array property, in bytes.
    pub max_array_raw_bytes: u64,
    /// Largest accepted decoded size of all array properties in one document.
    pub max_total_array_raw_bytes: u64,
}

impl Default for FbxDecodeLimits {
    fn default() -> Self {
        // Observed maxima across the ufbx corpus (683 binary files) and the
        // Mixamo/Three.js assets the browser app loads:
        //
        //   file bytes            7,045,584     nodes                 15,039
        //   depth                         8     properties per node    4,224
        //   blob bytes              328,086     string bytes          87,207
        //   array raw bytes       1,661,184     array elements       207,648
        //   total array bytes     7,352,416
        //
        // Note `properties per node` alone rules out a 4096 ceiling, which a
        // plausible-looking guess would have picked.
        Self {
            max_file_bytes: 1 << 30, // 1 GiB
            max_depth: 64,           // ufbx stops at 32
            max_nodes: 8_000_000,
            max_properties_per_node: 1 << 16,
            max_string_bytes: 16 << 20, // 16 MiB
            max_blob_bytes: 128 << 20,  // 128 MiB, ~400x observed
            max_array_elements: 1 << 28,
            max_array_raw_bytes: 256 << 20,     // 256 MiB
            max_total_array_raw_bytes: 1 << 30, // 1 GiB
        }
    }
}

impl FbxDecodeLimits {
    /// Limits raised well beyond [`Self::default`], for trusted local input.
    pub fn permissive() -> Self {
        Self {
            max_file_bytes: 16 << 30,
            max_depth: 256,
            max_nodes: 64_000_000,
            max_properties_per_node: 1 << 20,
            max_string_bytes: 256 << 20,
            max_blob_bytes: 2 << 30,
            max_array_elements: 1 << 32,
            max_array_raw_bytes: 4 << 30,
            max_total_array_raw_bytes: 8 << 30,
        }
    }

    /// Tight limits for fuzzing, so a reported allocation failure is a real
    /// bug rather than the fuzzer feeding a legitimately huge header.
    pub fn fuzzing() -> Self {
        Self {
            max_file_bytes: 1 << 20,
            max_depth: 16,
            max_nodes: 64_000,
            max_properties_per_node: 4096,
            max_string_bytes: 1 << 20,
            max_blob_bytes: 1 << 20,
            max_array_elements: 1 << 20,
            max_array_raw_bytes: 4 << 20,
            max_total_array_raw_bytes: 4 << 20,
        }
    }
}

// `#[non_exhaustive]` blocks `..Default::default()` for downstream crates, so
// the type would be unconfigurable outside this crate without these setters.
macro_rules! limit_setter {
    ($name:ident, $field:ident, $ty:ty, $doc:expr) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(mut self, value: $ty) -> Self {
            self.$field = value;
            self
        }
    };
}

impl FbxDecodeLimits {
    limit_setter!(
        with_max_file_bytes,
        max_file_bytes,
        u64,
        "Sets [`Self::max_file_bytes`]."
    );
    limit_setter!(with_max_depth, max_depth, u32, "Sets [`Self::max_depth`].");
    limit_setter!(with_max_nodes, max_nodes, u64, "Sets [`Self::max_nodes`].");
    limit_setter!(
        with_max_properties_per_node,
        max_properties_per_node,
        u64,
        "Sets [`Self::max_properties_per_node`]."
    );
    limit_setter!(
        with_max_string_bytes,
        max_string_bytes,
        u64,
        "Sets [`Self::max_string_bytes`]."
    );
    limit_setter!(
        with_max_blob_bytes,
        max_blob_bytes,
        u64,
        "Sets [`Self::max_blob_bytes`]."
    );
    limit_setter!(
        with_max_array_elements,
        max_array_elements,
        u64,
        "Sets [`Self::max_array_elements`]."
    );
    limit_setter!(
        with_max_array_raw_bytes,
        max_array_raw_bytes,
        u64,
        "Sets [`Self::max_array_raw_bytes`]."
    );
    limit_setter!(
        with_max_total_array_raw_bytes,
        max_total_array_raw_bytes,
        u64,
        "Sets [`Self::max_total_array_raw_bytes`]."
    );
}

/// How the FBX reader treats a document: what it may allocate, and how
/// strictly it enforces the binary container layout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FbxReadOptions {
    /// Allocation ceilings for this read.
    pub limits: FbxDecodeLimits,
    /// Reject anything the FBX binary layout does not strictly permit.
    ///
    /// Off by default: shipping exporters emit slop that every practical
    /// reader tolerates, so strictness is opt-in for validation tools.
    pub strict: bool,
}

impl FbxReadOptions {
    /// Options that reject any deviation from the documented layout.
    pub fn strict() -> Self {
        Self {
            strict: true,
            ..Self::default()
        }
    }

    /// Replaces the allocation ceilings.
    #[must_use]
    pub fn with_limits(mut self, limits: FbxDecodeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Enables or disables strict container validation.
    #[must_use]
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_order_decodes_both_layouts() {
        let bytes = [0x00, 0x00, 0x1d, 0x4c];
        assert_eq!(FbxByteOrder::Big.u32(bytes), 7500);
        assert_eq!(FbxByteOrder::Little.u32([0x4c, 0x1d, 0x00, 0x00]), 7500);
    }

    #[test]
    fn little_endian_swap_is_a_no_op() {
        let mut data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        FbxByteOrder::Little.swap_elements_in_place(&mut data, 4);
        assert_eq!(data, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn big_endian_swap_reverses_each_element() {
        let mut data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        FbxByteOrder::Big.swap_elements_in_place(&mut data, 4);
        assert_eq!(data, [4, 3, 2, 1, 8, 7, 6, 5]);
    }

    #[test]
    fn single_byte_elements_are_never_swapped() {
        let mut data = [1u8, 2, 3];
        FbxByteOrder::Big.swap_elements_in_place(&mut data, 1);
        assert_eq!(data, [1, 2, 3]);
    }

    #[test]
    fn setters_work_through_non_exhaustive() {
        let limits = FbxDecodeLimits::default()
            .with_max_depth(8)
            .with_max_nodes(9);
        assert_eq!(limits.max_depth, 8);
        assert_eq!(limits.max_nodes, 9);
    }

    #[test]
    fn defaults_accept_the_measured_corpus_maxima() {
        // Guards the calibration above: if someone tightens a default below
        // what real assets need, this fails instead of the browser app.
        let limits = FbxDecodeLimits::default();
        assert!(limits.max_file_bytes >= 7_045_584);
        assert!(limits.max_nodes >= 15_039);
        assert!(limits.max_depth >= 8);
        assert!(limits.max_properties_per_node >= 4_224);
        assert!(limits.max_blob_bytes >= 328_086);
        assert!(limits.max_string_bytes >= 87_207);
        assert!(limits.max_array_raw_bytes >= 1_661_184);
        assert!(limits.max_array_elements >= 207_648);
        assert!(limits.max_total_array_raw_bytes >= 7_352_416);
    }
}
