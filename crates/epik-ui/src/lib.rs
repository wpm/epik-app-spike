//! Leptos frontend for the Epik desktop application.
//!
//! The frontend defines no event or payload types of its own. Everything it
//! deserializes is an `epik-core` type, reached through this crate's dependency
//! on that crate with `default-features = false` — the types without the tokio
//! runtime that cannot build for wasm. There is therefore no mirrored copy of
//! `SessionEvent` here to fall out of step with the host's.

pub mod app;
pub mod components;
pub mod doctor_screen;
pub mod markdown;
pub mod ipc;
pub mod session;
pub mod transcript;

pub use app::App;
