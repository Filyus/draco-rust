//! Interning face corners into mesh points.
//!
//! Two readers turn a stream of face corners into a point list, and they do it
//! the same way: every corner names a key, corners sharing a key become one
//! point, and the points are numbered in the order their keys first appear.
//! Only the key differs -- FBX has no vertex identity across its layer
//! elements, so it keys on the attribute bits themselves, while OBJ is given
//! identity by the file and keys on the `v/vt/vn` triple.
//!
//! What the two do *after* interning has nothing in common: FBX walks its
//! layers to emit attributes, OBJ keeps the parsed references and resolves them
//! later. So this owns the interning alone, which is the part that was written
//! twice and the part where the hasher choice matters.
//!
//! PLY and STL do not intern at all. PLY is handed an explicit vertex list, and
//! STL carries no vertex identity, so welding it would be the reader inventing
//! one.

use std::collections::HashMap;
use std::hash::Hash;

/// Assigns dense point ids to corner keys, in order of first appearance.
///
/// The hasher is `foldhash` rather than the standard one, and the reason is the
/// keys: they are built from bytes of the file being read, so their
/// distribution belongs to whoever wrote it. `foldhash` seeds itself per
/// instance, so crafted collisions cannot be computed ahead of the run, and it
/// is `hashbrown`'s own default, which makes it the best-exercised fast hasher
/// available. A fixed-constant multiply hash -- `FxHash` and its kin -- would
/// be faster still and would hand an attacker closed-form collisions: measured
/// on two-word keys, 20k crafted ones cost 140 ms against 0 ms for the same
/// count drawn at random, and 80k cost 2.9 s. `std`'s SipHash resists that too,
/// but costs a quarter of the FBX weld's time on large meshes.
pub(crate) struct CornerWeld<K> {
    seen: HashMap<K, u32, foldhash::fast::RandomState>,
}

impl<K: Eq + Hash> CornerWeld<K> {
    /// A weld sized for a mesh of `corners` corners.
    ///
    /// The count is an upper bound on the points it can produce -- every corner
    /// distinct -- so this trades a possibly oversized table for the growth
    /// rehashes a mesh that welds little would otherwise pay.
    pub(crate) fn with_capacity(corners: usize) -> Self {
        Self {
            seen: HashMap::with_capacity_and_hasher(corners, Default::default()),
        }
    }

    /// The point id for `key`, and whether this call is what created it.
    ///
    /// The caller records its own value for a new point: this holds the keys
    /// only for as long as it takes to recognise them again.
    pub(crate) fn intern(&mut self, key: K) -> (u32, bool) {
        let next = self.seen.len() as u32;
        match self.seen.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => (*entry.get(), false),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(next);
                (next, true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dense_and_follow_first_appearance() {
        let mut weld: CornerWeld<u32> = CornerWeld::with_capacity(4);

        assert_eq!(weld.intern(7), (0, true));
        assert_eq!(weld.intern(3), (1, true));
        assert_eq!(weld.intern(7), (0, false));
        assert_eq!(weld.intern(9), (2, true));
    }

    /// The ids may not depend on the hasher, which seeds itself per instance.
    #[test]
    fn two_welds_of_the_same_keys_agree() {
        let keys = [5u32, 1, 5, 2, 1, 9];
        let ids = |()| {
            let mut weld: CornerWeld<u32> = CornerWeld::with_capacity(keys.len());
            keys.iter().map(|k| weld.intern(*k).0).collect::<Vec<_>>()
        };

        assert_eq!(ids(()), ids(()));
        assert_eq!(ids(()), vec![0, 1, 0, 2, 1, 3]);
    }
}
