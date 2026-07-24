//! WASM binary entry point for embyr-admin-ui.
//!
//! Compiled only with the `csr` feature (trunk build --release).
//! Host-target tests use the library (lib.rs); this binary is not compiled
//! in host test mode because it requires the `csr` feature.

fn main() {
    #[cfg(feature = "csr")]
    {
        console_error_panic_hook::set_once();
        leptos::mount::mount_to_body(embyr_admin_ui::app::App);
    }
}
