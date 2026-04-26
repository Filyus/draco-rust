pub fn disable_noisy_debug_env() {
    // Some environments set this globally; it causes corner table routines in the
    // C++ Draco library (and our Rust port) to spam stdout/stderr and distort
    // benchmarks.
    //
    // Note: In newer Rust versions, mutating process environment is `unsafe`
    // because it can race with other threads. Our tests call this early.
    unsafe {
        std::env::remove_var("DRACO_CT_DEBUG");
    }
}

#[allow(dead_code)]
pub fn skip_if_cpp_bridge_unavailable() -> bool {
    if draco_cpp_test_bridge::is_available() {
        false
    } else {
        eprintln!("SKIPPING: C++ test bridge not available");
        true
    }
}
