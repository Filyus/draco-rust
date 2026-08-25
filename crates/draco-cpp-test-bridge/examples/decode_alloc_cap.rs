//! Decode one file under an allocator that refuses any single allocation over
//! a cap and prints a backtrace for it.
//!
//! Names the allocation site of a fuzz OOM without a debugger: the refusal
//! happens at the call, so the backtrace is the decode path that asked.
//!
//! ```text
//! CAP_BYTES=1000000 cargo run --release -p draco-cpp-test-bridge --example decode_alloc_cap -- oom.bin
//! ```
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Capped;

static CAP: AtomicUsize = AtomicUsize::new(usize::MAX);
static LARGEST: AtomicUsize = AtomicUsize::new(0);
static REPORTING: AtomicBool = AtomicBool::new(false);

// SAFETY: every method forwards to `System` with the layout and pointer
// unchanged; the cap only decides whether to call it or return null, which
// `GlobalAlloc` permits as allocation failure.
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        LARGEST.fetch_max(size, Ordering::Relaxed);
        if size > CAP.load(Ordering::Relaxed) && !REPORTING.swap(true, Ordering::Relaxed) {
            // Printing allocates, so the flag keeps this from recursing.
            eprintln!("REFUSED a single allocation of {size} bytes ({:.2} GiB)", size as f64 / (1024.0 * 1024.0 * 1024.0));
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
            REPORTING.store(false, Ordering::Relaxed);
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LARGEST.fetch_max(new_size, Ordering::Relaxed);
        if new_size > CAP.load(Ordering::Relaxed) && !REPORTING.swap(true, Ordering::Relaxed) {
            eprintln!("REFUSED a realloc to {new_size} bytes ({:.2} GiB)", new_size as f64 / (1024.0 * 1024.0 * 1024.0));
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
            REPORTING.store(false, Ordering::Relaxed);
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Capped = Capped;

fn main() {
    let path = std::env::args().nth(1).expect("usage: decode_alloc_cap <file>");
    let cap: usize = std::env::var("CAP_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64 * 1024 * 1024);
    let data = std::fs::read(&path).expect("read input");
    println!("input: {} bytes, cap {cap} bytes", data.len());
    CAP.store(cap, Ordering::Relaxed);

    {
        use draco_core::decoder_buffer::DecoderBuffer;
        use draco_core::mesh::Mesh;
        use draco_core::mesh_decoder::MeshDecoder;
        let mut buffer = DecoderBuffer::new(&data);
        let mut mesh = Mesh::new();
        let result = MeshDecoder::new().decode(&mut buffer, &mut mesh);
        CAP.store(usize::MAX, Ordering::Relaxed);
        println!("as mesh: {result:?}");
        CAP.store(cap, Ordering::Relaxed);
    }
    {
        use draco_core::decoder_buffer::DecoderBuffer;
        use draco_core::point_cloud::PointCloud;
        use draco_core::point_cloud_decoder::PointCloudDecoder;
        let mut buffer = DecoderBuffer::new(&data);
        let mut point_cloud = PointCloud::new();
        let result = PointCloudDecoder::new().decode(&mut buffer, &mut point_cloud);
        CAP.store(usize::MAX, Ordering::Relaxed);
        println!("as point cloud: {result:?}");
    }

    println!("largest single allocation attempted: {} bytes", LARGEST.load(Ordering::Relaxed));
}
