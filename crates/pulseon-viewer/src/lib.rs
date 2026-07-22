//! Renderer-independent state and native reads for the PulseOn desktop viewer.

#![forbid(unsafe_code)]

pub mod core;
pub mod model;
pub mod query;
mod source;
pub mod worker;

pub use source::SourceError;
