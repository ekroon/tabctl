//! Native host top-layer API surface.
//!
//! This crate root remains declarative and forwards to implementation modules.

mod host_impl;

pub use host_impl::run;
