use std::path::Path;

/// Shown only if the app is built without its frontend. Not a fallback anyone
/// should see in normal use — it exists so that failure is legible rather than a
/// blank window.
const PLACEHOLDER: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Epik</title></head>
<body style="font:14px system-ui;padding:2rem;background:#0a0a0a;color:#ededed">
<h1 style="font-size:1rem">Frontend not built</h1>
<p>Run <code>trunk build</code> in <code>crates/epik-ui</code>, then rebuild.</p>
</body></html>
"#;

fn main() {
    // `tauri::generate_context!` embeds `frontendDist` at compile time and fails
    // outright if the directory is missing. The real contents come from
    // `trunk build` in crates/epik-ui, which writes here. This placeholder only
    // ensures that a plain `cargo build` on a fresh checkout — CI's
    // `cargo build --workspace`, for instance — is not a hard error just because
    // the wasm frontend has not been built yet.
    let dist = Path::new("dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("create the frontend dist directory");
        std::fs::write(dist.join("index.html"), PLACEHOLDER).expect("write the dist placeholder");
    }
    tauri_build::build()
}
