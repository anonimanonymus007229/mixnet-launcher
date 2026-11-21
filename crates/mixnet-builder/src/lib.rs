//! CLI integration is deliberate: the user can select any compatible ddmix.exe.
pub mod mods;
mod pipeline;
pub mod repository;
pub mod tools;
pub use pipeline::{Event, start};
