//! CLI top-layer API surface.
//!
//! This module stays declarative and forwards into implementation modules.

mod impls;

pub use impls::run;
