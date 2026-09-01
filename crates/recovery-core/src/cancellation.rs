use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::error::{RecoveryError, RecoveryResult};

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> RecoveryResult<()> {
        if self.is_cancelled() {
            Err(RecoveryError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared() {
        let token = CancellationToken::default();
        let clone = token.clone();
        clone.cancel();
        assert_eq!(token.check(), Err(RecoveryError::Cancelled));
    }
}
