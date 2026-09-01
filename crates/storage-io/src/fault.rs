use std::sync::atomic::{AtomicUsize, Ordering};
use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use crate::BlockDevice;

#[derive(Clone, Debug)]
pub enum Fault {
    Transient { fail_calls: usize },
    Permanent,
    Disconnect,
    PartialRead { max_bytes: usize },
}

pub struct FaultInjectingDevice<D> {
    inner: D,
    fault: Fault,
    calls: AtomicUsize,
}

impl<D> FaultInjectingDevice<D> {
    pub fn new(inner: D, fault: Fault) -> Self {
        Self { inner, fault, calls: AtomicUsize::new(0) }
    }
}

impl<D: BlockDevice> BlockDevice for FaultInjectingDevice<D> {
    fn capacity(&self) -> u64 { self.inner.capacity() }

    fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            Fault::Permanent => return Err(RecoveryError::IoFailure("injected permanent failure".into())),
            Fault::Disconnect => return Err(RecoveryError::IoFailure("injected disconnect".into())),
            Fault::Transient { fail_calls } if call < fail_calls => return Err(RecoveryError::IoFailure("injected transient failure".into())),
            Fault::PartialRead { max_bytes } => {
                let requested = usize::try_from(range.length).map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
                let actual = requested.min(max_bytes);
                let partial = ByteRange::new(range.offset, actual as u64)?;
                return self.inner.read(partial, output);
            }
            _ => {}
        }
        self.inner.read(range, output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MemoryDevice(Vec<u8>);
    impl BlockDevice for MemoryDevice {
        fn capacity(&self)->u64 { self.0.len() as u64 }
        fn read(&self,r:ByteRange,o:&mut[u8])->RecoveryResult<usize> {
            r.validate_within(self.capacity())?;
            let n=usize::try_from(r.length).unwrap();
            o[..n].copy_from_slice(&self.0[r.offset as usize..r.end().unwrap() as usize]); Ok(n)
        }
    }
    #[test] fn transient_fault_recovers_after_budget(){let d=FaultInjectingDevice::new(MemoryDevice(b"abcd".to_vec()),Fault::Transient{fail_calls:1});let mut o=[0;2];assert!(d.read(ByteRange::new(0,2).unwrap(),&mut o).is_err());assert_eq!(d.read(ByteRange::new(0,2).unwrap(),&mut o).unwrap(),2);}
    #[test] fn partial_fault_limits_read(){let d=FaultInjectingDevice::new(MemoryDevice(b"abcd".to_vec()),Fault::PartialRead{max_bytes:1});let mut o=[0;2];assert_eq!(d.read(ByteRange::new(0,2).unwrap(),&mut o).unwrap(),1);assert_eq!(o[0],b'a');}
}
