//! Typed, transport-neutral request validation for the macOS privileged helper.
//!
//! The actual XPC transport and platform client authentication remain in the
//! native helper target. This module deliberately contains no privilege
//! escalation or arbitrary path access. It gives both sides a small schema
//! that can be validated before any privileged operation is attempted.

#![cfg(target_os = "macos")]

use recovery_core::{RecoveryError, RecoveryResult};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_READ_LENGTH: u64 = 8 * 1024 * 1024;
pub const MAX_SOURCE_REF_LENGTH: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolRequest {
    pub version: u16,
    pub operation: Operation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    GetSourceMetadata {
        source_ref: String,
    },
    OpenReadOnly {
        source_ref: String,
    },
    Read {
        handle: u64,
        offset: u64,
        length: u64,
    },
    Close {
        handle: u64,
    },
}

impl ProtocolRequest {
    pub fn validate(&self) -> RecoveryResult<()> {
        if self.version != PROTOCOL_VERSION {
            return Err(RecoveryError::Unsupported(
                "unsupported macOS helper protocol version".into(),
            ));
        }
        match &self.operation {
            Operation::GetSourceMetadata { source_ref }
            | Operation::OpenReadOnly { source_ref } => validate_source_ref(source_ref),
            Operation::Read { handle, length, .. } => {
                if *handle == 0 {
                    return Err(RecoveryError::IoFailure(
                        "invalid macOS helper handle".into(),
                    ));
                }
                if *length == 0 || *length > MAX_READ_LENGTH {
                    return Err(RecoveryError::IoFailure(
                        "macOS helper read length is outside the allowed range".into(),
                    ));
                }
                Ok(())
            }
            Operation::Close { handle } => {
                if *handle == 0 {
                    return Err(RecoveryError::IoFailure(
                        "invalid macOS helper handle".into(),
                    ));
                }
                Ok(())
            }
        }
    }
}

fn validate_source_ref(source_ref: &str) -> RecoveryResult<()> {
    if source_ref.is_empty() || source_ref.len() > MAX_SOURCE_REF_LENGTH {
        return Err(RecoveryError::IoFailure(
            "macOS helper source reference has invalid length".into(),
        ));
    }
    if source_ref.bytes().any(|byte| byte == 0) {
        return Err(RecoveryError::IoFailure(
            "macOS helper source reference contains NUL".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_read_within_limit() {
        let request = ProtocolRequest {
            version: PROTOCOL_VERSION,
            operation: Operation::Read {
                handle: 1,
                offset: 0,
                length: MAX_READ_LENGTH,
            },
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn rejects_oversized_reads() {
        let request = ProtocolRequest {
            version: PROTOCOL_VERSION,
            operation: Operation::Read {
                handle: 1,
                offset: 0,
                length: MAX_READ_LENGTH + 1,
            },
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn rejects_wrong_version_and_invalid_source_ref() {
        let wrong_version = ProtocolRequest {
            version: PROTOCOL_VERSION + 1,
            operation: Operation::GetSourceMetadata {
                source_ref: "disk4".into(),
            },
        };
        assert!(wrong_version.validate().is_err());
        let nul_ref = ProtocolRequest {
            version: PROTOCOL_VERSION,
            operation: Operation::OpenReadOnly {
                source_ref: "disk\0".into(),
            },
        };
        assert!(nul_ref.validate().is_err());
    }

    #[test]
    fn rejects_zero_handles() {
        let request = ProtocolRequest {
            version: PROTOCOL_VERSION,
            operation: Operation::Close { handle: 0 },
        };
        assert!(request.validate().is_err());
    }
}
