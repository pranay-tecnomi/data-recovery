use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{read_exact_at, Diagnostic, DiscoveryResult, DiskGeometry, PartitionCandidate};

const GPT_HEADER_SIZE: usize = 92;
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const PARTITION_ENTRY_MIN_SIZE: u32 = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GptHeader {
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub partition_entries_lba: u64,
    pub partition_entry_count: u32,
    pub partition_entry_size: u32,
}

fn le_u32(bytes: &[u8]) -> u32 { u32::from_le_bytes(bytes.try_into().expect("fixed slice")) }
fn le_u64(bytes: &[u8]) -> u64 { u64::from_le_bytes(bytes.try_into().expect("fixed slice")) }

pub fn parse_gpt_header(bytes: &[u8]) -> RecoveryResult<GptHeader> {
    if bytes.len() < GPT_HEADER_SIZE || &bytes[..8] != GPT_SIGNATURE {
        return Err(RecoveryError::IoFailure("invalid GPT signature".into()));
    }
    let header_size = le_u32(&bytes[12..16]);
    if header_size < GPT_HEADER_SIZE as u32 || header_size as usize > bytes.len() {
        return Err(RecoveryError::IoFailure("invalid GPT header size".into()));
    }
    let partition_entry_count = le_u32(&bytes[80..84]);
    let partition_entry_size = le_u32(&bytes[84..88]);
    if partition_entry_count == 0 || partition_entry_size < PARTITION_ENTRY_MIN_SIZE || partition_entry_size % 8 != 0 {
        return Err(RecoveryError::IoFailure("invalid GPT partition entry geometry".into()));
    }
    Ok(GptHeader {
        current_lba: le_u64(&bytes[24..32]),
        backup_lba: le_u64(&bytes[32..40]),
        first_usable_lba: le_u64(&bytes[40..48]),
        last_usable_lba: le_u64(&bytes[48..56]),
        partition_entries_lba: le_u64(&bytes[72..80]),
        partition_entry_count,
        partition_entry_size,
    })
}

pub fn discover_gpt<D: BlockDevice>(device: &D, geometry: DiskGeometry) -> RecoveryResult<DiscoveryResult> {
    let sector_len = usize::try_from(geometry.logical_sector_size)
        .map_err(|_| RecoveryError::LengthTooLarge { length: geometry.logical_sector_size })?;
    let mut sector = vec![0u8; sector_len];
    read_exact_at(device, geometry.logical_sector_size, &mut sector)?;
    let header = parse_gpt_header(&sector)?;
    if header.current_lba != 1 {
        return Err(RecoveryError::IoFailure("unexpected GPT primary header location".into()));
    }
    if header.first_usable_lba > header.last_usable_lba {
        return Err(RecoveryError::IoFailure("invalid GPT usable range".into()));
    }
    let table_bytes = u64::from(header.partition_entry_count)
        .checked_mul(u64::from(header.partition_entry_size))
        .ok_or(RecoveryError::RangeOverflow)?;
    if table_bytes > 16 * 1024 * 1024 {
        return Err(RecoveryError::LengthTooLarge { length: table_bytes });
    }
    let table_offset = header.partition_entries_lba
        .checked_mul(geometry.logical_sector_size)
        .ok_or(RecoveryError::RangeOverflow)?;
    let table_len = usize::try_from(table_bytes)
        .map_err(|_| RecoveryError::LengthTooLarge { length: table_bytes })?;
    let mut table = vec![0u8; table_len];
    read_exact_at(device, table_offset, &mut table)?;

    let mut result = DiscoveryResult::default();
    let entry_size = header.partition_entry_size as usize;
    for index in 0..header.partition_entry_count as usize {
        let start = index.checked_mul(entry_size).ok_or(RecoveryError::RangeOverflow)?;
        let entry = &table[start..start + entry_size];
        if entry[..16].iter().all(|b| *b == 0) { continue; }
        let first_lba = le_u64(&entry[32..40]);
        let last_lba = le_u64(&entry[40..48]);
        if first_lba > last_lba || first_lba < header.first_usable_lba || last_lba > header.last_usable_lba {
            result.diagnostics.push(Diagnostic::GptRangeInvalid { index: index as u32 });
            continue;
        }
        let sector_count = last_lba.checked_sub(first_lba).and_then(|v| v.checked_add(1)).ok_or(RecoveryError::RangeOverflow)?;
        let offset = first_lba.checked_mul(geometry.logical_sector_size).ok_or(RecoveryError::RangeOverflow)?;
        let length = sector_count.checked_mul(geometry.logical_sector_size).ok_or(RecoveryError::RangeOverflow)?;
        let range = ByteRange::new(offset, length)?;
        if range.validate_within(device.capacity()).is_err() {
            result.diagnostics.push(Diagnostic::RangeOutOfBounds { index: index as u8 });
            continue;
        }
        result.partitions.push(PartitionCandidate { index: index as u8, range, type_code: 0xEE, bootable: false });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_minimal_header() {
        let mut b = vec![0u8; 512];
        b[..8].copy_from_slice(b"EFI PART");
        b[12..16].copy_from_slice(&(92u32).to_le_bytes());
        b[24..32].copy_from_slice(&(1u64).to_le_bytes());
        b[32..40].copy_from_slice(&(99u64).to_le_bytes());
        b[40..48].copy_from_slice(&(34u64).to_le_bytes());
        b[48..56].copy_from_slice(&(98u64).to_le_bytes());
        b[72..80].copy_from_slice(&(2u64).to_le_bytes());
        b[80..84].copy_from_slice(&(4u32).to_le_bytes());
        b[84..88].copy_from_slice(&(128u32).to_le_bytes());
        assert_eq!(parse_gpt_header(&b).unwrap().partition_entry_count, 4);
    }
}
