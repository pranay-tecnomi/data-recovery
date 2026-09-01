#[derive(Debug, Eq, PartialEq)]
pub enum RecoveryError {
    RangeOverflow,
    OutOfRange { offset: u64, length: u64, capacity: u64 },
    Cancelled,
    PermissionDenied,
    IoFailure(String),
}

pub type RecoveryResult<T> = Result<T, RecoveryError>;
