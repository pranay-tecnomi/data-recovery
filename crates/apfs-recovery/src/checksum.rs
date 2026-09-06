use recovery_core::{RecoveryError, RecoveryResult};

/// Compute the Fletcher-64 checksum used by APFS object blocks.
///
/// APFS stores the checksum in the first eight bytes of the object header;
/// those bytes are therefore excluded from the checksum input. The remaining
/// object bytes are interpreted as little-endian 32-bit words and accumulated
/// modulo 0xffffffff.
pub fn fletcher64(data: &[u8]) -> RecoveryResult<u64> {
    if data.len() < 8 || data.len() % 4 != 0 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }

    const MODULUS: u64 = 0xffff_ffff;
    let mut sum1 = 0u64;
    let mut sum2 = 0u64;

    for word in data[8..].chunks_exact(4) {
        let value = u32::from_le_bytes(word.try_into().expect("exact APFS checksum word")) as u64;
        sum1 += value;
        sum2 += sum1;
    }

    sum1 %= MODULUS;
    sum2 %= MODULUS;
    let lower = (sum1 + sum2) % MODULUS;
    let upper = (sum1 + lower) % MODULUS;
    Ok(lower | (upper << 32))
}

/// Verify the Fletcher-64 checksum stored in an APFS object header.
pub fn verify_fletcher64(data: &[u8]) -> RecoveryResult<()> {
    if data.len() < 8 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    let stored = u64::from_le_bytes(data[..8].try_into().expect("validated APFS checksum"));
    let computed = fletcher64(data)?;
    if stored != computed {
        return Err(RecoveryError::IoFailure(format!(
            "APFS object checksum mismatch: stored {stored:#018x}, computed {computed:#018x}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unaligned_object() {
        assert!(fletcher64(&[0u8; 10]).is_err());
    }

    #[test]
    fn checksum_round_trip() {
        let mut object = vec![0u8; 64];
        object[8..12].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        object[12..16].copy_from_slice(&0x5566_7788u32.to_le_bytes());
        let checksum = fletcher64(&object).unwrap();
        object[..8].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(fletcher64(&object).unwrap(), checksum);
        assert!(verify_fletcher64(&object).is_ok());
    }

    #[test]
    fn detects_corruption() {
        let mut object = vec![0u8; 64];
        let checksum = fletcher64(&object).unwrap();
        object[..8].copy_from_slice(&checksum.to_le_bytes());
        object[20] = 1;
        assert!(verify_fletcher64(&object).is_err());
    }
}
