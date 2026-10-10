//! quipu-server's global allocator (aegis-67p0lj).

/// aegis-67p0lj: glibc malloc kept the server's freed transients resident (one
/// arena held 1.34G, all of it free) until the memory recycler restarted it.
/// jemalloc purges dirty pages after `dirty_decay_ms` on a background thread,
/// so a burst is returned instead of becoming the new floor.
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// The settings measured in the lab A/B (aegis-67p0lj). jemalloc
/// reads this symbol at startup; `MALLOC_CONF` in the environment still
/// overrides it.
///
/// The crate denies `unsafe_code`; exporting a symbol is unsafe only because a
/// name clash would be undefined behaviour. `malloc_conf` is the one name
/// jemalloc reserves for exactly this, and the value is a
/// NUL-terminated static byte string that is never written.
#[allow(unsafe_code)]
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8] = b"background_thread:true,dirty_decay_ms:1000,muzzy_decay_ms:1000\0";
