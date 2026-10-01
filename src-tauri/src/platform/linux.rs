//! Linux rendering setup.

// WebKitGTK's DMA-BUF renderer fails to create an EGL display on wlroots
// compositors, NVIDIA's proprietary driver, and minimal sessions, and the
// compositor cannot be probed reliably up front. Disabling it is the value that
// works everywhere, so it is the default; a user on a known-good stack can opt
// back into the hardware path with WEBKIT_DISABLE_DMABUF_RENDERER=0.
pub(crate) fn configure_linux_rendering() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: called from `run` before any window or WebKitGTK thread
        // exists, so nothing can read the environment concurrently.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
}
