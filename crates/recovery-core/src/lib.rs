#![forbid(unsafe_code)]

pub mod cancellation;
pub mod candidate;
pub mod error;
pub mod extent;
pub mod ids;
pub mod range;

pub use cancellation::CancellationToken;
pub use candidate::{
    Completeness, Confidence, Evidence, FileCandidate, Origin, Validation,
};
pub use error::{RecoveryError, RecoveryResult};
pub use extent::{total_length, validate_logical_layout, Extent};
pub use ids::{CandidateId, RecoveryJobId, ScanSessionId, SourceId};
pub use range::ByteRange;
