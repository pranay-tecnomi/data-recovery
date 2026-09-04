//! Bounded file carving.
//!
//! Recovers candidates directly from raw bytes when filesystem metadata is
//! missing or unreliable. Signatures are hints: every candidate carries the
//! evidence behind its boundary, and a carve whose end cannot be established is
//! reported as partial rather than guessed at.

#![forbid(unsafe_code)]

pub mod registry;
pub mod scanner;

pub use registry::{
    max_lookbehind, BoundaryStrategy, CarveLimits, Signature, GIF, JPEG, PDF, PNG, REGISTRY, ZIP,
};
pub use scanner::carve;
