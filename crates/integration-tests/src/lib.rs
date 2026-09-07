//! Fixture builders for end-to-end tests.
//!
//! These construct real on-disk structures rather than mocks, so the tests
//! exercise the same parsing paths a genuine image would.

#![forbid(unsafe_code)]

pub mod apfs_fixture;
pub mod fixture;

pub use fixture::{Fat32Image, MemoryDevice};
