# Known-failing fuzz inputs

Reproducers for bugs that are understood but not yet fixed, kept per target in
`known_failures/<target>/`.

They live here rather than in `fuzz/seeds/` because `fuzz/seed_corpus.sh` copies
everything under `seeds/` into the corpus. A known-failing input placed there
does not report the bug once -- it ends every campaign of that target on its
first execution, and ends the corpus replay after it, so the run stops before it
has fuzzed anything. That is how one such reproducer turned a soak red without
adding a single fact.

Nothing reads this directory. It is where a reproducer waits: when the bug is
fixed, move the file into `fuzz/seeds/<target>/`, where it becomes a seed that
holds the fix down.
