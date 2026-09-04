//! exFAT recovery: boot geometry, allocation bitmap, directory entry sets.
//!
//! Every structure here is parsed from untrusted on-disk metadata. Derived
//! offsets are checked and bounds-validated before any read, and no parser
//! allocates from a length it has not validated.

#![forbid(unsafe_code)]

pub mod bitmap;
pub mod boot;
pub mod directory;

pub use bitmap::AllocationBitmap;
pub use boot::{parse_volume, ExfatVolume, FIRST_CLUSTER};
pub use directory::{
    read_directory, DirectoryEntry, EntrySetError, ATTR_DIRECTORY,
};

#[cfg(test)]
pub(crate) mod testimage;
