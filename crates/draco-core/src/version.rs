//! Draco bitstream version constants and checks.
//!
//! Version numbers for the mesh and point-cloud bitstreams, the versions at
//! which format features were introduced, and helpers such as
//! [`version_at_least`] used to gate version-dependent decode/encode paths.

// Draco bitstream version constants.
//
// This module defines version-related constants for the Draco bitstream format.
// These constants are used for encoding/decoding version checks and default settings.

// =============================================================================
// Current/Latest Draco Bitstream Versions
// =============================================================================
// Note: Mesh and PointCloud have different latest versions in the C++ Draco.
// See src/draco/compression/config/compression_shared.h

/// Latest major version of the Draco Point Cloud bitstream.
pub const DRACO_POINT_CLOUD_BITSTREAM_VERSION_MAJOR: u8 = 2;

/// Latest minor version of the Draco Point Cloud bitstream.
pub const DRACO_POINT_CLOUD_BITSTREAM_VERSION_MINOR: u8 = 3;

/// Latest major version of the Draco Mesh bitstream.
pub const DRACO_MESH_BITSTREAM_VERSION_MAJOR: u8 = 2;

/// Latest minor version of the Draco Mesh bitstream.
pub const DRACO_MESH_BITSTREAM_VERSION_MINOR: u8 = 2;

// =============================================================================
// Default Encoder Versions (by encoding method)
// =============================================================================
// These use the latest supported versions for each geometry type.

/// Default version for Mesh encoding (both Sequential and Edgebreaker).
/// Uses the latest mesh bitstream version (v2.2).
pub const DEFAULT_MESH_VERSION: (u8, u8) = (
    DRACO_MESH_BITSTREAM_VERSION_MAJOR,
    DRACO_MESH_BITSTREAM_VERSION_MINOR,
);

/// Default version for PointCloud encoding, both methods.
///
/// Upstream picks the version from the geometry type alone -- see
/// `PointCloudEncoder::EncodeHeader`, which writes
/// `kDracoPointCloudBitstreamVersion` for anything whose encoder type is
/// `POINT_CLOUD` -- so the sequential and KD-tree methods share it. An earlier
/// comment here claimed C++ wrote 1.3 for sequential point clouds; it does not,
/// and writing 1.3 also turned the attribute count into a `u32` where upstream
/// writes a varint.
pub const DEFAULT_POINT_CLOUD_VERSION: (u8, u8) = (
    DRACO_POINT_CLOUD_BITSTREAM_VERSION_MAJOR,
    DRACO_POINT_CLOUD_BITSTREAM_VERSION_MINOR,
);

// =============================================================================
// Milestone Versions (for feature checks)
// =============================================================================

/// Version that introduced header flags field (v1.3).
/// From this version onwards, the header includes a 16-bit flags field.
pub const VERSION_FLAGS_INTRODUCED: (u8, u8) = (1, 3);

/// Version that introduced varint encoding for metadata fields (v2.0).
/// Before this, num_faces/num_points/num_attributes used fixed u32.
pub const VERSION_VARINT_ENCODING: (u8, u8) = (2, 0);

/// Version that introduced varint for unique_id in attributes (v1.3).
/// Before v1.3, unique_id was encoded as u16.
pub const VERSION_VARINT_UNIQUE_ID: (u8, u8) = (1, 3);

// =============================================================================
// Utility Functions
// =============================================================================

/// Checks if the given version is at least the target version.
/// Returns true if (major, minor) >= (target_major, target_minor).
#[inline]
pub fn version_at_least(major: u8, minor: u8, target: (u8, u8)) -> bool {
    major > target.0 || (major == target.0 && minor >= target.1)
}

/// Checks if the given version is less than the target version.
/// Returns true if (major, minor) < (target_major, target_minor).
#[inline]
pub fn version_less_than(major: u8, minor: u8, target: (u8, u8)) -> bool {
    major < target.0 || (major == target.0 && minor < target.1)
}

/// Checks if the given version uses varint encoding for metadata fields.
/// Returns true for v2.0+.
#[inline]
pub fn uses_varint_encoding(major: u8, _minor: u8) -> bool {
    major >= VERSION_VARINT_ENCODING.0
}

/// Checks if the given version includes header flags.
/// Returns true for v1.3+.
#[inline]
pub fn has_header_flags(major: u8, minor: u8) -> bool {
    version_at_least(major, minor, VERSION_FLAGS_INTRODUCED)
}

/// Checks if the given version uses varint for attribute unique_id.
/// Returns true for v1.3+.
#[inline]
pub fn uses_varint_unique_id(major: u8, minor: u8) -> bool {
    version_at_least(major, minor, VERSION_VARINT_UNIQUE_ID)
}

/// Oldest bitstream version this crate encodes, matching the documented
/// compatibility floor of Draco 1.0.0.
pub const OLDEST_ENCODABLE_VERSION: (u8, u8) = (1, 0);

/// Rejects a target bitstream version the encoder cannot actually write.
///
/// `set_version` takes two free bytes, and every version-dependent branch in
/// the encoders asks a predicate such as [`has_header_flags`] about them. Those
/// predicates answer for any input, so a version nobody supports does not fail:
/// it silently selects an older header layout for some fields and the current
/// one for others, and the result is a stream this crate's own decoder cannot
/// read. Validating once, where the version enters, keeps that from being
/// discovered field by field.
///
/// `(0, 0)` means "use the default" and is accepted here; the encoders
/// substitute their own default for it.
pub fn validate_encodable_version(
    major: u8,
    minor: u8,
    latest: (u8, u8),
) -> Result<(), crate::status::DracoError> {
    if major == 0 && minor == 0 {
        return Ok(());
    }
    if version_less_than(major, minor, OLDEST_ENCODABLE_VERSION) {
        return Err(crate::status::DracoError::UnsupportedVersion(format!(
            "Cannot encode bitstream version {major}.{minor}: oldest supported is {}.{}",
            OLDEST_ENCODABLE_VERSION.0, OLDEST_ENCODABLE_VERSION.1
        )));
    }
    if bitstream_version(major, minor) > bitstream_version(latest.0, latest.1) {
        return Err(crate::status::DracoError::UnsupportedVersion(format!(
            "Cannot encode bitstream version {major}.{minor}: newest supported is {}.{}",
            latest.0, latest.1
        )));
    }
    Ok(())
}

/// Packs a `(major, minor)` bitstream version into the single `0xMMmm` value
/// used for ordered comparisons such as `bitstream_version(maj, min) < 0x0202`.
/// Centralizes the encoding so call sites cannot transpose major/minor.
#[inline]
pub fn bitstream_version(major: u8, minor: u8) -> u16 {
    ((major as u16) << 8) | (minor as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_at_least() {
        // v1.3 >= v1.3 = true
        assert!(version_at_least(1, 3, (1, 3)));
        // v2.0 >= v1.3 = true
        assert!(version_at_least(2, 0, (1, 3)));
        // v1.2 >= v1.3 = false
        assert!(!version_at_least(1, 2, (1, 3)));
        // v0.9 >= v1.3 = false
        assert!(!version_at_least(0, 9, (1, 3)));
    }

    #[test]
    fn test_version_less_than() {
        // v1.2 < v2.0 = true
        assert!(version_less_than(1, 2, (2, 0)));
        // v2.0 < v2.0 = false
        assert!(!version_less_than(2, 0, (2, 0)));
        // v2.1 < v2.0 = false
        assert!(!version_less_than(2, 1, (2, 0)));
    }

    #[test]
    fn test_uses_varint_encoding() {
        assert!(!uses_varint_encoding(1, 3));
        assert!(uses_varint_encoding(2, 0));
        assert!(uses_varint_encoding(2, 2));
    }

    #[test]
    fn test_has_header_flags() {
        assert!(!has_header_flags(1, 2));
        assert!(has_header_flags(1, 3));
        assert!(has_header_flags(2, 0));
    }

    #[test]
    fn test_bitstream_version() {
        assert_eq!(bitstream_version(2, 2), 0x0202);
        assert_eq!(bitstream_version(1, 1), 0x0101);
        assert_eq!(bitstream_version(2, 0), 0x0200);
        // The packed form must preserve version ordering.
        assert!(bitstream_version(1, 2) < bitstream_version(2, 0));
        assert!(bitstream_version(2, 1) < bitstream_version(2, 2));
    }
}
