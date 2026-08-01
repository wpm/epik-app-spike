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

use epik_core::SessionEvent;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
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

/// A `tauri::ipc::Channel`, from the frontend's side.
///
/// The host writes `SessionEvent`s onto it and they arrive here as JSON, which is
/// then deserialized into `epik_core::SessionEvent` — the same type, by the same
/// derived impl, that the host serialized. A channel rather than a global event:
/// events belong to one session, and `emit` would broadcast them to every
/// listener with no way to tell whose session they came from.
pub struct EventChannel {
    js: JsValue,
    /// The closure the host calls. Dropping it would detach the handler, so the
    /// channel owns it for as long as it lives.
    _on_message: Closure<dyn FnMut(JsValue)>,
}

impl EventChannel {
    /// Build a channel whose messages are handed to `on_event`, already decoded.
    ///
    /// Undecodable messages go to `on_error` rather than being dropped: a message
    /// this crate cannot parse means the host and the frontend disagree about a
    /// type, which is a bug worth surfacing rather than a glitch to swallow.
    pub fn new(
        mut on_event: impl FnMut(SessionEvent) + 'static,
        mut on_error: impl FnMut(String) + 'static,
    ) -> Result<Self, IpcError> {
        let js = construct_channel()?;
        let on_message = Closure::wrap(Box::new(move |message: JsValue| {
            match from_js::<SessionEvent>(message) {
                Ok(event) => on_event(event),
                Err(err) => on_error(err.to_string()),
            }
        }) as Box<dyn FnMut(JsValue)>);
        js_sys::Reflect::set(
            &js,
            &JsValue::from_str("onmessage"),
            on_message.as_ref().unchecked_ref(),
        )
        .map_err(|_| IpcError::Decode("could not attach the channel handler".to_owned()))?;
        Ok(Self {
            js,
            _on_message: on_message,
        })
    }
}

/// `new window.__TAURI__.core.Channel()`, via reflection.
///
/// Reached through `Reflect` rather than a `#[wasm_bindgen]` constructor binding
/// because the class only exists once the Tauri script has run; a static binding
/// would be a link error in a plain browser instead of the `NoHost` this returns.
fn construct_channel() -> Result<JsValue, IpcError> {
    let get = |target: &JsValue, key: &str| {
        js_sys::Reflect::get(target, &JsValue::from_str(key)).map_err(|_| IpcError::NoHost)
    };
    let core = get(&get(&js_sys::global(), "__TAURI__")?, "core")?;
    let ctor: js_sys::Function = get(&core, "Channel")?
        .dyn_into()
        .map_err(|_| IpcError::NoHost)?;
    js_sys::Reflect::construct(&ctor, &js_sys::Array::new())
        .map_err(|_| IpcError::Decode("could not construct an IPC channel".to_owned()))
}

/// Call a command, passing an event channel alongside the usual arguments.
///
/// The channel has to be the live JS object, so the argument object is assembled
/// rather than produced wholesale from JSON: `args` contributes its fields, then
/// the channel is set on top under the name the command's parameter uses.
pub async fn call_with_channel<T: DeserializeOwned>(
    cmd: &str,
    args: &impl Serialize,
    channel: &EventChannel,
) -> Result<T, IpcError> {
    if !host_available() {
        return Err(IpcError::NoHost);
    }
    let object = js_sys::Object::new();
    let from_args: js_sys::Object = to_js(args)?
        .dyn_into()
        .map_err(|_| IpcError::Decode("arguments did not encode to an object".to_owned()))?;
    js_sys::Object::assign(&object, &from_args);
    js_sys::Reflect::set(&object, &JsValue::from_str("channel"), &channel.js)
        .map_err(|_| IpcError::Decode("could not attach the channel".to_owned()))?;

    match invoke(cmd, object.into()).await {
        Ok(value) => from_js(value),
        Err(err) => Err(command_error(err)),
    }
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
        Err(err) => Err(command_error(err)),
    }
}

/// A rejected `invoke` carries the `Err` value the command returned — a string,
/// for every command in this app.
fn command_error(err: JsValue) -> IpcError {
    IpcError::Command(
        err.as_string()
            .or_else(|| js_sys::JSON::stringify(&err).ok().map(String::from))
            .unwrap_or_else(|| "command failed".to_owned()),
    )
}
