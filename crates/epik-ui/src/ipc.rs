//! The frontend's side of the IPC boundary.
//!
//! Commands are called through `window.__TAURI__.core.invoke`, which
//! `withGlobalTauri` puts on the window. Reaching for the global rather than the
//! `@tauri-apps/api` npm package keeps this crate's toolchain to cargo and
//! trunk — no node, no package.json, no second dependency graph to keep in step
//! with the Rust one.
//!
//! Arguments and results cross as JSON text and are converted with `serde_json`,
//! deliberately. It would be faster to hand `serde_wasm_bindgen` the values
//! directly, but going through JSON means the frontend uses *the same*
//! `Serialize`/`Deserialize` implementations the host used — the ones derived on
//! `epik-core`'s own types. That is what makes a type mismatch a compile error
//! here instead of a silently-missing field at runtime.

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

/// Anything that can go wrong calling a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    /// `window.__TAURI__` is absent — the app is running in a plain browser
    /// (`trunk serve`) rather than inside the Tauri window.
    NoHost,
    /// The command ran and returned an error.
    Command(String),
    /// The command's result did not match the type expected of it. A bug rather
    /// than a condition, so it says so.
    Decode(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHost => write!(
                f,
                "not running inside the Epik window (window.__TAURI__ is missing)"
            ),
            Self::Command(message) => write!(f, "{message}"),
            Self::Decode(message) => write!(f, "unexpected response shape: {message}"),
        }
    }
}

/// Whether the Tauri host is reachable at all. Checked before invoking so
/// running under `trunk serve` produces one clear message instead of a failure
/// per command.
pub fn host_available() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("__TAURI__"))
        .map(|tauri| !tauri.is_undefined() && !tauri.is_null())
        .unwrap_or(false)
}

fn to_js(args: &impl Serialize) -> Result<JsValue, IpcError> {
    let json = serde_json::to_string(args).map_err(|e| IpcError::Decode(e.to_string()))?;
    js_sys::JSON::parse(&json).map_err(|_| IpcError::Decode(format!("could not encode {json}")))
}

fn from_js<T: DeserializeOwned>(value: JsValue) -> Result<T, IpcError> {
    // A command returning `()` comes back as `undefined`, which `JSON.stringify`
    // turns into nothing at all; `null` is what serde expects for a unit.
    if value.is_undefined() {
        return serde_json::from_str("null").map_err(|e| IpcError::Decode(e.to_string()));
    }
    let json = js_sys::JSON::stringify(&value)
        .map(String::from)
        .map_err(|_| IpcError::Decode("result was not JSON-representable".to_owned()))?;
    serde_json::from_str(&json).map_err(|e| IpcError::Decode(format!("{e}: {json}")))
}

/// Call a command with no arguments.
pub async fn call<T: DeserializeOwned>(cmd: &str) -> Result<T, IpcError> {
    call_with(cmd, &serde_json::Map::new()).await
}

/// Call a command with arguments. `args` serializes to the object Tauri expects,
/// so its field names must match the command's parameter names.
pub async fn call_with<T: DeserializeOwned>(
    cmd: &str,
    args: &impl Serialize,
) -> Result<T, IpcError> {
    if !host_available() {
        return Err(IpcError::NoHost);
    }
    match invoke(cmd, to_js(args)?).await {
        Ok(value) => from_js(value),
        // Command errors arrive as the `Err` string the command returned.
        Err(err) => Err(IpcError::Command(
            err.as_string()
                .or_else(|| js_sys::JSON::stringify(&err).ok().map(String::from))
                .unwrap_or_else(|| "command failed".to_owned()),
        )),
    }
}
