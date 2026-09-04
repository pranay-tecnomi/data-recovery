//! Recovery output: destination safety, streaming copy, and the audit manifest.
//!
//! Nothing here writes to a source. The destination gate runs before any job,
//! and every write targets the validated destination directory.

#![forbid(unsafe_code)]

pub mod destination;
pub mod manifest;
pub mod session;
pub mod writer;

pub use destination::{validate_destination, DestinationRejection, SafeDestination};
pub use manifest::{build_manifest, manifest_line};
pub use session::{
    checkpoint_path, Checkpoint, ResumeRejection, SourceFingerprint, SCHEMA_VERSION,
};
pub use writer::{
    recover_all, recover_candidate, CollisionPolicy, ItemOutcome, RecoveredItem,
};
