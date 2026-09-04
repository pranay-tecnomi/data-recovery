#[derive(Debug, Eq, PartialEq)]
pub enum RecoveryError {
    RangeOverflow,
    LengthTooLarge { length: u64 },
    OutOfRange { offset: u64, length: u64, capacity: u64 },
    OutputBufferTooSmall { required: usize, provided: usize },
    Cancelled,
    PermissionDenied,
    /// The source went away mid-operation (removable media unplugged).
    /// Resume requires revalidating source identity.
    Disconnected,
    /// A read that may succeed if retried; the scheduler owns retry policy.
    TransientReadFailure(String),
    /// A read that will not succeed on retry (media defect).
    PermanentReadFailure(String),
    /// The operation is not supported by this platform or source kind.
    Unsupported(String),
    IoFailure(String),
}

pub type RecoveryResult<T> = Result<T, RecoveryError>;
