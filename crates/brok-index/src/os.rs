//! What the index asks of the operating system.

/// Returns memory that was freed but is still held by the allocator.
///
/// glibc keeps freed memory in one pool per group of threads and reuses it
/// only there. Writing the index and embedding run on different threads, so
/// without this the second pass of indexing needed about 20% more memory at
/// its peak (brokspace/brok#200). Elsewhere this does nothing.
#[allow(unsafe_code)]
pub(crate) fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: takes no pointers and may be called from any thread.
    unsafe {
        libc::malloc_trim(0);
    }
}
