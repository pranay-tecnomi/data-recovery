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
    BoundaryStrategy, CarveLimits, GIF, JPEG, PDF, PNG, REGISTRY, Signature, ZIP, max_lookbehind,
};
pub use scanner::carve;
