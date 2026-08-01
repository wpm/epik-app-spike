//! Trunk's entry point. The application itself is in the library so it can be
//! checked and, later, tested without going through wasm-bindgen's `main`.

fn main() {
    // Without this, a panic in wasm is a bare "unreachable executed" in the
    // console with no location — the single highest-value line in a wasm app.
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(epik_ui::App);
}
