#![no_main]

use draco_io::{FbxDecodeLimits, FbxReadOptions, FbxScene};
use libfuzzer_sys::fuzz_target;

// Fuzzes the FBX *writer* using scenes the reader produced from mutated input.
//
// A reader-only campaign never exercises the writer, and hand-built scenes only
// cover shapes we thought to write down. Feeding back whatever the reader
// accepts reaches scenes with empty meshes, degenerate polygon streams,
// mismatched layer sets and skins whose clusters point nowhere.
//
// The re-read must succeed: anything we emit has to be readable by us. That is
// a stronger contract than byte identity, which FBX does not promise, and it is
// what the writer's strict-mode test asserts for hand-built scenes.
fuzz_target!(|data: &[u8]| {
    let options = FbxReadOptions::default().with_limits(FbxDecodeLimits::fuzzing());
    let Ok(scene) = FbxScene::from_bytes_with_options(data, options.clone()) else {
        return;
    };

    // Our own output is canonical, so it must satisfy strict validation. Use
    // permissive limits here: the scene came from tightly-limited input, but
    // re-encoding can legitimately exceed the fuzzing ceilings.
    let strict = FbxReadOptions::strict().with_limits(FbxDecodeLimits::permissive());

    // Binary and ASCII are two spellings of the same document, and only the
    // binary one used to be written here. The ASCII writer carries arithmetic
    // of its own -- the line-wrap budget, the float spelling, the base64
    // blocks -- and the 6100 pass below is a *different* writer, so it does not
    // stand in for this one.
    for spelling in [scene.to_bytes(), scene.to_ascii_bytes()] {
        let Ok(written) = spelling else {
            // The writer is allowed to refuse a scene it cannot represent, as
            // long as it says so instead of emitting corrupt bytes.
            continue;
        };

        let reread = FbxScene::from_bytes_with_options(&written, strict.clone())
            .expect("writer output must satisfy the reader's strict mode");

        assert_eq!(
            count_nodes(&scene),
            count_nodes(&reread),
            "node count changed across a write/read round-trip"
        );
        assert_eq!(
            scene.materials.len(),
            reread.materials.len(),
            "material count changed across a write/read round-trip"
        );
    }

    // The 6100 object model is a second writer over the same scene, reached by
    // no other fuzz target: name-keyed identity, the root-key collision guard,
    // and the Takes/Key state machine all only run on this path.
    for legacy in [scene.to_legacy_bytes(), scene.to_legacy_ascii_bytes()] {
        let Ok(written) = legacy else {
            // Refusing a skin/blend shape (or an ASCII name collision) is
            // allowed; emitting corrupt bytes is not.
            continue;
        };
        let reread = FbxScene::from_bytes_with_options(&written, strict.clone())
            .expect("6100 writer output must satisfy the reader's strict mode");
        assert_eq!(
            count_nodes(&scene),
            count_nodes(&reread),
            "node count changed across a 6100 write/read round-trip"
        );
    }
});

fn count_nodes(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        1 + node.children.iter().map(visit).sum::<usize>()
    }
    scene.root_nodes.iter().map(visit).sum()
}
