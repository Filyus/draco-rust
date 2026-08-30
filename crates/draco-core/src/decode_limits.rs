//! Caller-set ceilings on what one decode may produce.
//!
//! [`decode_budget`](crate::decode_budget) and this module answer different
//! questions and both are needed. The budget is a backstop against a
//! *reservation nothing backs* -- a header naming gigabytes over a stream that
//! carries none of them -- and it is always on. These are the caller's policy
//! on *how large a decode may legitimately be*, and they count what the file
//! honestly describes as well as what it lies about.
//!
//! The reason the budget cannot do this job is measured: legitimate geometry
//! reaches four to five orders of magnitude more output than input, because a
//! constant attribute entropy-codes to a size independent of its count. A
//! stream of under a kilobyte decodes six million points, and refusing it
//! would be the interoperability bug `decode_budget` exists to have removed.
//! No ratio separates that from a hostile claim, so the only honest instrument
//! is an absolute ceiling, and only the caller knows where it sits.
//!
//! `SECURITY.md` records that this decoder does not cap reconstructed geometry
//! by design. That stays true of the format; what changes is that the caller
//! can now say otherwise, and the default says it for them.

use crate::status::{DracoError, ErrorKind, Status};

/// Ceilings on one decode, applied to what the stream reconstructs.
///
/// Every field is a hard ceiling: exceeding one fails the decode with
/// [`ErrorKind::LimitExceeded`], which a caller can tell apart from
/// [`ErrorKind::AllocationExceedsInput`] -- its own policy refusing a large
/// file, rather than the decoder refusing a malformed one.
///
/// The counts are the *decoded* ones, not the ones a source file was authored
/// with. Draco splits a point wherever an attribute seam runs through it, and
/// the growth is real: measured across ten assets it runs from zero to 7.7%,
/// and one exporter's 65,532-point primitive decodes to 76,742. A ceiling
/// derived from what an exporter writes would refuse files that follow its own
/// convention.
///
/// Install with [`DecoderBuffer::with_limits`](crate::decoder_buffer::DecoderBuffer::with_limits).
///
/// ```
/// use draco_core::{DecodeLimits, DecoderBuffer};
///
/// let stream = [0u8; 0];
/// let buffer = DecoderBuffer::new(&stream)
///     .with_limits(DecodeLimits::default().with_max_points(1_000_000));
/// # let _ = buffer;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeLimits {
    /// Largest accepted decoded point count.
    pub max_points: u64,
    /// Largest accepted decoded face count.
    pub max_faces: u64,
    /// Largest accepted total size of decoded attribute values, in bytes.
    ///
    /// This is the one that bounds memory. The other two are proxies whose
    /// relationship to bytes the attribute set moves around: of two measured
    /// assets, the one with a third fewer points held twice the bytes per
    /// primitive, because it carried tangents and five texture-coordinate sets
    /// against a position and a normal.
    pub max_decoded_bytes: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        // Largest seen in one decoded stream across a corpus of real assets:
        // 25,000,000 points and 700 MB (a photogrammetric point cloud) and
        // 6,618,864 faces (an unchunked OBJ). These sit an order of magnitude
        // above that, because their job is to stop an absurd header rather
        // than to police legitimate files -- a caller with a real memory
        // budget sets its own.
        Self {
            max_points: 256_000_000,
            max_faces: 512_000_000,
            max_decoded_bytes: 2 << 30,
        }
    }
}

impl DecodeLimits {
    /// No ceiling at all, which is what this crate did before the type existed.
    ///
    /// For a caller that decodes trusted local input and would rather have a
    /// scan loaded whole than a refusal.
    pub fn permissive() -> Self {
        Self {
            max_points: u64::MAX,
            max_faces: u64::MAX,
            max_decoded_bytes: u64::MAX,
        }
    }

    /// Tight ceilings for fuzzing, so a reported allocation failure is a real
    /// bug rather than the fuzzer feeding a legitimately huge count.
    pub fn fuzzing() -> Self {
        Self {
            max_points: 1 << 20,
            max_faces: 1 << 20,
            max_decoded_bytes: 64 << 20,
        }
    }

    pub(crate) fn check_points(self, points: u64) -> Status {
        Self::check("points", points, self.max_points)
    }

    pub(crate) fn check_faces(self, faces: u64) -> Status {
        Self::check("faces", faces, self.max_faces)
    }

    pub(crate) fn check_decoded_bytes(self, bytes: u64) -> Status {
        Self::check("decoded attribute bytes", bytes, self.max_decoded_bytes)
    }

    fn check(what: &str, value: u64, ceiling: u64) -> Status {
        if value > ceiling {
            return Err(DracoError::new(
                ErrorKind::LimitExceeded,
                format!("stream decodes {value} {what}, over the caller's ceiling of {ceiling}"),
            ));
        }
        Ok(())
    }
}

// `#[non_exhaustive]` blocks `..Default::default()` for downstream crates, so
// the type would be unconfigurable outside this crate without these setters.
macro_rules! limit_setter {
    ($name:ident, $field:ident, $doc:expr) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(mut self, value: u64) -> Self {
            self.$field = value;
            self
        }
    };
}

impl DecodeLimits {
    limit_setter!(with_max_points, max_points, "Sets [`Self::max_points`].");
    limit_setter!(with_max_faces, max_faces, "Sets [`Self::max_faces`].");
    limit_setter!(
        with_max_decoded_bytes,
        max_decoded_bytes,
        "Sets [`Self::max_decoded_bytes`]."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_clear_every_asset_the_calibration_measured() {
        let limits = DecodeLimits::default();
        // The largest single stream of each kind, from
        // `dev/docs/decode-limits-calibration.md`.
        assert!(limits.check_points(25_000_000).is_ok());
        assert!(limits.check_faces(6_618_864).is_ok());
        assert!(limits.check_decoded_bytes(700_000_000).is_ok());
    }

    #[test]
    fn the_defaults_refuse_the_claim_that_motivated_them() {
        let limits = DecodeLimits::default();
        // The `decode_drc` artifact's header, and what it would occupy as
        // three-component `f32` positions.
        assert!(limits.check_points(1_073_741_828).is_err());
        assert!(limits.check_decoded_bytes(1_073_741_828 * 12).is_err());
    }

    #[test]
    fn permissive_limits_refuse_nothing() {
        let limits = DecodeLimits::permissive();
        assert!(limits.check_points(u64::MAX).is_ok());
        assert!(limits.check_faces(u64::MAX).is_ok());
        assert!(limits.check_decoded_bytes(u64::MAX).is_ok());
    }

    #[test]
    fn a_refusal_names_the_caller_rather_than_the_file() {
        let error = DecodeLimits::default()
            .check_points(u64::MAX)
            .expect_err("over the ceiling");
        assert_eq!(error.kind(), ErrorKind::LimitExceeded);
        assert!(error.message().contains("ceiling"), "{error}");
    }
}

/// End to end: the ceilings refuse a stream this crate wrote, and the defaults
/// do not.
///
/// The arithmetic tests above pin the comparison; these pin the wiring, which
/// is the part that can silently stop existing.
#[cfg(all(test, feature = "encoder", feature = "decoder"))]
mod wiring {
    use super::*;
    use crate::decoder_buffer::DecoderBuffer;
    use crate::draco_types::DataType;
    use crate::encoder_buffer::EncoderBuffer;
    use crate::encoder_options::EncoderOptions;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use crate::point_cloud::PointCloud;
    use crate::point_cloud_decoder::PointCloudDecoder;
    use crate::point_cloud_encoder::PointCloudEncoder;

    const NUM_POINTS: usize = 100_000;

    fn a_stream_of_100k_points() -> Vec<u8> {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(NUM_POINTS);
        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            NUM_POINTS,
        );
        point_cloud.add_attribute(position);

        let mut options = EncoderOptions::new();
        options.set_attribute_int(0, "quantization_bits", 8);
        let mut encoded = EncoderBuffer::new();
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(point_cloud);
        encoder.encode(&options, &mut encoded).expect("encode");
        encoded.data().to_vec()
    }

    fn decode_under(limits: DecodeLimits, stream: &[u8]) -> Result<usize, DracoError> {
        let mut buffer = DecoderBuffer::new(stream).with_limits(limits);
        let mut decoded = PointCloud::new();
        PointCloudDecoder::new().decode(&mut buffer, &mut decoded)?;
        Ok(decoded.num_points())
    }

    #[test]
    fn the_defaults_decode_a_stream_this_crate_wrote() {
        let stream = a_stream_of_100k_points();
        assert_eq!(
            decode_under(DecodeLimits::default(), &stream).expect("within the defaults"),
            NUM_POINTS
        );
    }

    #[test]
    fn a_point_ceiling_below_the_header_refuses_it() {
        let stream = a_stream_of_100k_points();
        let error = decode_under(DecodeLimits::default().with_max_points(1_000), &stream)
            .expect_err("over the ceiling");
        assert_eq!(error.kind(), ErrorKind::LimitExceeded);
        assert!(error.message().contains("points"), "{error}");
    }

    /// The byte ceiling is the one that bounds memory, and it is charged from
    /// the attribute layout rather than from the point count: three `f32`
    /// components over 100,000 points is 1.2 MB whatever the stream costs.
    #[test]
    fn a_byte_ceiling_below_the_attribute_layout_refuses_it() {
        let stream = a_stream_of_100k_points();
        let error = decode_under(
            DecodeLimits::default().with_max_decoded_bytes(NUM_POINTS as u64 * 12 - 1),
            &stream,
        )
        .expect_err("over the ceiling");
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "{error}");
        assert!(
            error.message().contains("decoded attribute bytes"),
            "{error}"
        );

        assert_eq!(
            decode_under(
                DecodeLimits::default().with_max_decoded_bytes(NUM_POINTS as u64 * 12),
                &stream
            )
            .expect("exactly the layout fits"),
            NUM_POINTS
        );
    }

    /// The face ceiling sits on the connectivity, which neither of the other
    /// two reaches: a mesh can be far more faces than points.
    #[test]
    fn a_face_ceiling_below_the_connectivity_refuses_it() {
        use crate::mesh::Mesh;
        use crate::mesh_decoder::MeshDecoder;
        use crate::mesh_encoder::MeshEncoder;

        const SIDE: u32 = 64;
        let mut mesh = Mesh::new();
        let points = (SIDE * SIDE) as usize;
        mesh.set_num_points(points);
        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            points,
        );
        mesh.add_attribute(position);
        let mut faces = Vec::new();
        for y in 0..SIDE - 1 {
            for x in 0..SIDE - 1 {
                let a = y * SIDE + x;
                faces.push([a, a + 1, a + SIDE]);
                faces.push([a + 1, a + SIDE + 1, a + SIDE]);
            }
        }
        mesh.try_set_num_faces(faces.len()).expect("faces");
        for (index, face) in faces.iter().enumerate() {
            mesh.set_face_from_indices(index, *face);
        }
        let num_faces = faces.len();

        let mut encoded = EncoderBuffer::new();
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        encoder
            .encode(&EncoderOptions::new(), &mut encoded)
            .expect("encode");
        let stream = encoded.data().to_vec();

        let decode = |limits: DecodeLimits| {
            let mut buffer = DecoderBuffer::new(&stream).with_limits(limits);
            let mut decoded = Mesh::new();
            MeshDecoder::new()
                .decode(&mut buffer, &mut decoded)
                .map(|()| decoded.num_faces())
        };

        assert_eq!(
            decode(DecodeLimits::default()).expect("within the defaults"),
            num_faces
        );
        let error = decode(DecodeLimits::default().with_max_faces(num_faces as u64 - 1))
            .expect_err("over the ceiling");
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "{error}");
        assert!(error.message().contains("faces"), "{error}");
    }

    #[test]
    fn permissive_limits_decode_what_the_defaults_do() {
        let stream = a_stream_of_100k_points();
        assert_eq!(
            decode_under(DecodeLimits::permissive(), &stream).expect("no ceiling"),
            NUM_POINTS
        );
    }
}
