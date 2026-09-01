#[derive(Debug, Eq, PartialEq)]
pub enum RecoveryError {
    RangeOverflow,
    LengthTooLarge { length: u64 },
    OutOfRange { offset: u64, length: u64, capacity: u64 },
    OutputBufferTooSmall { required: usize, provided: usize },
    Cancelled,
    PermissionDenied,
    IoFailure(String),
}

pub type RecoveryResult<T> = Result<T, RecoveryError>;
