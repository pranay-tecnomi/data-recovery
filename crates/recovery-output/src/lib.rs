//! Recovery output: destination safety, streaming copy, and the audit manifest.
//!
//! Nothing here writes to a source. The destination gate runs before any job,
//! and every write targets the validated destination directory.

#![forbid(unsafe_code)]

pub mod destination;
pub mod manifest;
pub mod session;
pub mod writer;

pub use destination::{DestinationRejection, SafeDestination, validate_destination};
pub use manifest::{build_manifest, manifest_line};
pub use session::{
    Checkpoint, ResumeRejection, SCHEMA_VERSION, SourceFingerprint, checkpoint_path,
};
pub use writer::{CollisionPolicy, ItemOutcome, RecoveredItem, recover_all, recover_candidate};
