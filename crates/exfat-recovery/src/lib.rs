//! exFAT recovery: boot geometry, allocation bitmap, directory entry sets.
//!
//! Every structure here is parsed from untrusted on-disk metadata. Derived
//! offsets are checked and bounds-validated before any read, and no parser
//! allocates from a length it has not validated.

#![forbid(unsafe_code)]

pub mod bitmap;
pub mod boot;
pub mod directory;
pub mod extents;

pub use bitmap::AllocationBitmap;
pub use boot::{ExfatVolume, FIRST_CLUSTER, parse_volume};
pub use directory::{ATTR_DIRECTORY, DirectoryEntry, EntrySetError, cluster_chain, read_directory};
pub use extents::{
    Confidence, DeletedCandidate, ExtentState, StreamExtents, deleted_candidate, stream_extents,
};

#[cfg(test)]
pub(crate) mod testimage;
