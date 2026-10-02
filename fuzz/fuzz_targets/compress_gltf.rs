#![no_main]

//! Fuzz the document-preserving glTF compressor on untrusted glTF/GLB bytes.
//!
//! Parsing and transformation must never panic, abort, or read external files.
//! This entry point has no resolver, so external resource URIs are refused
//! rather than resolved from the filesystem.
//!
//! Every primitive the compressor accepts is then read back, and three things
//! must hold, whatever the input:
//!
//! 1. The compressed primitive decodes. The compressor wrote the stream itself,
//!    so a failure other than a decode limit is a bug.
//! 2. Every accessor it declares -- attributes, indices, morph targets -- counts
//!    what the stream decodes to. `DracoOnly` declares them from the encoder's
//!    description without decoding, so this is the only place a wrong count
//!    shows: the file would describe data it does not hold.
//! 3. In `Fallback`, the uncompressed copy holds exactly the decoded data.
//!
//! Each input is compressed twice on separate copies of the document: once with
//! the default options, and once with options derived from a hash of the input,
//! so the mode, speed and quantization vary without changing the input format
//! the corpus is built from.
//!
//! Each pass compresses the first [`MAX_PRIMITIVES`] primitives and leaves the
//! rest. A primitive's compression depends on the ones before it only through
//! what they left in the document -- accessors already compressed, buffer views
//! remapped -- which the first few already exercise, and the corpus is
//! dominated by mutations of one fixture with fifty primitives over the same
//! accessors: 119 of its 530 glTF inputs hold 78% of its primitives. Taking
//! all of them cost the campaign most of its time on the fiftieth copy of the
//! same work.

use draco_core::decode_limits::DecodeLimits;
use draco_core::ErrorKind;
use draco_gltf::{
    import_slice_with_options, ComponentType, CompressionMode, CompressionOptions,
    DocumentAccessorSource, Error, Import, ImportOptions, MeshIndex, PackedGeometry,
    PrimitiveIndex, QuantizationBits, ValidationProfile, KHR_DRACO_MESH_COMPRESSION,
};
use libfuzzer_sys::fuzz_target;

/// The primitives each pass compresses, in document order.
const MAX_PRIMITIVES: usize = 8;

fuzz_target!(|data: &[u8]| {
    // `DecodeLimits::fuzzing()` is deliberately far tighter than the shipped
    // defaults: the glTF container does not bound the geometry a Draco stream
    // reconstructs, so a header naming a hundred million points is a legal
    // multi-gigabyte decode. Under the shipped defaults that decode is a
    // legitimate `-rss_limit_mb` trip, and real findings drown in that noise.
    let options = ImportOptions {
        profile: ValidationProfile::Gltf21Draft,
        draco_decode_limits: DecodeLimits::fuzzing(),
        ..ImportOptions::default()
    };
    let Ok(import) = import_slice_with_options(data, &options) else {
        return;
    };
    for compression in [CompressionOptions::default(), derived_options(data)] {
        compress_and_check(import.clone(), &compression, data);
    }
});

fn compress_and_check(mut import: Import, options: &CompressionOptions, input: &[u8]) {
    let mut attempts = 0;
    for mesh in 0..import.document.meshes().len() {
        let primitive_count = import
            .document
            .mesh(MeshIndex(mesh))
            .map_or(0, |mesh| mesh.primitive_count());
        for primitive in 0..primitive_count {
            if attempts == MAX_PRIMITIVES {
                return;
            }
            attempts += 1;
            let already_draco = import
                .document
                .primitive(MeshIndex(mesh), primitive)
                .is_some_and(|reference| reference.extension(KHR_DRACO_MESH_COMPRESSION).is_some());
            if import
                .compress_primitive(MeshIndex(mesh), primitive, options.clone())
                .is_err()
                || already_draco
            {
                continue;
            }
            check_compressed(&import, MeshIndex(mesh), primitive, options, input);
        }
    }
}

fn check_compressed(
    import: &Import,
    mesh: MeshIndex,
    primitive: usize,
    options: &CompressionOptions,
    input: &[u8],
) {
    let fail = |what: String| -> ! {
        panic!(
            "compressed primitive {}/{primitive} {what}\noptions: mode={:?} speed={}/{} \
             quantization={:?}\ninput: {}",
            usize::from(mesh),
            options.mode,
            options.encoding_speed,
            options.decoding_speed,
            options.quantization,
            hex(input)
        )
    };

    // Oracle 1: the stream the compressor wrote decodes.
    let decoded = match import.read_primitive(PrimitiveIndex::new(mesh, primitive)) {
        Ok(decoded) => decoded,
        Err(error) if is_limit(&error) => return,
        Err(error) => fail(format!("does not read back: {error}")),
    };

    let Some(reference) = import.document.primitive(mesh, primitive) else {
        fail("disappeared from the document".into());
    };
    let source = DocumentAccessorSource::new(&import.document, &import.resources);
    let count_of = |accessor: usize| {
        import
            .document
            .accessor(accessor.into())
            .and_then(|accessor| accessor.count())
    };

    // Oracle 2: every declared count is what the stream decodes to.
    let vertices = decoded.vertex_count() as u64;
    for (semantic, accessor) in reference.attribute_indices() {
        let count = count_of(accessor.into());
        if count != Some(vertices) {
            fail(format!(
                "declares {semantic} with {count:?} elements, the stream decodes {vertices}"
            ));
        }
    }
    let decoded_indices = decoded.indices().map(|indices| indices.count() as u64);
    let declared_indices = reference
        .indices()
        .and_then(|accessor| count_of(accessor.into()));
    if declared_indices != decoded_indices {
        fail(format!(
            "declares {declared_indices:?} indices, the stream decodes {decoded_indices:?}"
        ));
    }
    for (target, attributes) in reference.morph_targets().enumerate() {
        for (semantic, value) in attributes {
            let count = value
                .as_u64()
                .and_then(|accessor| count_of(accessor as usize));
            if count != Some(vertices) {
                fail(format!(
                    "declares morph target {target} {semantic} with {count:?} elements, \
                     the stream decodes {vertices}"
                ));
            }
        }
    }

    // Oracle 3: in `Fallback`, the ordinary accessors hold the decoded data.
    if options.mode == CompressionMode::Fallback {
        if let Err(what) = check_fallback_copy(&source, &decoded, reference) {
            fail(what);
        }
    }
}

fn check_fallback_copy(
    source: &DocumentAccessorSource<'_>,
    decoded: &PackedGeometry,
    reference: draco_gltf::PrimitiveRef<'_>,
) -> Result<(), String> {
    let accessors: Vec<(&str, usize)> = reference
        .attribute_indices()
        .map(|(semantic, accessor)| (semantic, accessor.into()))
        .collect();
    for attribute in decoded.attributes() {
        let Some(&(_, accessor)) = accessors
            .iter()
            .find(|(semantic, _)| *semantic == attribute.semantic())
        else {
            return Err(format!("Fallback copy has no {}", attribute.semantic()));
        };
        let copy = match source.read_accessor(accessor) {
            Ok(copy) => copy,
            Err(error) if is_limit(&error) => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "Fallback copy of {} does not read: {error}",
                    attribute.semantic()
                ))
            }
        };
        if ComponentType::from_gltf(copy.component_type.into()) == Some(attribute.component_type())
            && copy.bytes != attribute.bytes()
        {
            return Err(format!(
                "Fallback copy of {} differs from the decoded stream",
                attribute.semantic()
            ));
        }
    }
    if let (Some(indices), Some(accessor)) = (decoded.indices(), reference.indices()) {
        let copy = match source.read_accessor(accessor.into()) {
            Ok(copy) => copy,
            Err(error) if is_limit(&error) => return Ok(()),
            Err(error) => return Err(format!("Fallback index copy does not read: {error}")),
        };
        let widen = |bytes: &[u8], width: usize| -> Vec<u32> {
            bytes
                .chunks_exact(width)
                .map(|chunk| {
                    let mut word = [0u8; 4];
                    word[..width].copy_from_slice(chunk);
                    u32::from_le_bytes(word)
                })
                .collect()
        };
        let decoded_width = indices.bytes().len() / indices.count().max(1);
        let copy_width = copy.bytes.len() / copy.count.max(1);
        if (1..=4).contains(&decoded_width)
            && (1..=4).contains(&copy_width)
            && widen(indices.bytes(), decoded_width) != widen(&copy.bytes, copy_width)
        {
            return Err("Fallback index copy differs from the decoded stream".into());
        }
    }
    Ok(())
}

/// Options picked by a hash of the whole input, so the fuzzer reaches every
/// mode, speed and quantization without the input carrying them explicitly.
fn derived_options(data: &[u8]) -> CompressionOptions {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in data {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    let mut take = |range: u64| {
        let value = hash % range;
        hash /= range;
        value
    };
    let mode = if take(2) == 0 {
        CompressionMode::Fallback
    } else {
        CompressionMode::DracoOnly
    };
    let speed = take(11) as u8;
    let mut bits = || match take(4) {
        0 => None,
        _ => Some(1 + take(30) as u8),
    };
    CompressionOptions {
        mode,
        encoding_speed: speed,
        decoding_speed: speed,
        quantization: QuantizationBits {
            position: bits(),
            normal: bits(),
            tex_coord: bits(),
            color: bits(),
            generic: bits(),
        },
        ..CompressionOptions::default()
    }
}

/// A refusal to decode past a limit, which is the limit working, not a bug.
fn is_limit(error: &Error) -> bool {
    match error {
        Error::ResourceLimit(_) => true,
        Error::Decode(error) => matches!(
            error.kind(),
            ErrorKind::LimitExceeded | ErrorKind::AllocationExceedsInput
        ),
        _ => false,
    }
}

/// Hex of the whole fuzz input, printed with an oracle failure: libFuzzer on
/// Windows/MSVC does not get to write its artifact when a Rust panic aborts.
fn hex(input: &[u8]) -> String {
    input.iter().map(|b| format!("{b:02x}")).collect()
}
