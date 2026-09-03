use crc32fast::Hasher;
use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{read_exact_at, Diagnostic, DiscoveryResult, DiskGeometry, PartitionCandidate};

const GPT_HEADER_SIZE: usize = 92;
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const PARTITION_ENTRY_MIN_SIZE: u32 = 128;
const MAX_TABLE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GptHeader {
    pub header_size: u32,
    pub header_crc32: u32,
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub partition_entries_lba: u64,
    pub partition_entry_count: u32,
    pub partition_entry_size: u32,
    pub partition_entries_crc32: u32,
}

fn le_u32(bytes: &[u8]) -> u32 { u32::from_le_bytes(bytes.try_into().expect("fixed slice")) }
fn le_u64(bytes: &[u8]) -> u64 { u64::from_le_bytes(bytes.try_into().expect("fixed slice")) }

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

pub fn validate_gpt_header_crc(bytes: &[u8], header_size: usize, expected: u32) -> RecoveryResult<()> {
    if header_size < GPT_HEADER_SIZE || header_size > bytes.len() {
        return Err(RecoveryError::IoFailure("invalid GPT header size for CRC".into()));
    }
    let mut copy = bytes[..header_size].to_vec();
    copy[16..20].fill(0);
    if crc32(&copy) != expected {
        return Err(RecoveryError::IoFailure("GPT header CRC mismatch".into()));
    }
    Ok(())
}

pub fn parse_gpt_header(bytes: &[u8]) -> RecoveryResult<GptHeader> {
    if bytes.len() < GPT_HEADER_SIZE || &bytes[..8] != GPT_SIGNATURE {
        return Err(RecoveryError::IoFailure("invalid GPT signature".into()));
    }
    let header_size = le_u32(&bytes[12..16]);
    if header_size < GPT_HEADER_SIZE as u32 || header_size as usize > bytes.len() {
        return Err(RecoveryError::IoFailure("invalid GPT header size".into()));
    }
    let header_crc32 = le_u32(&bytes[16..20]);
    validate_gpt_header_crc(bytes, header_size as usize, header_crc32)?;
    let partition_entry_count = le_u32(&bytes[80..84]);
    let partition_entry_size = le_u32(&bytes[84..88]);
    if partition_entry_count == 0 || partition_entry_size < PARTITION_ENTRY_MIN_SIZE || partition_entry_size % 8 != 0 {
        return Err(RecoveryError::IoFailure("invalid GPT partition entry geometry".into()));
    }
    Ok(GptHeader {
        header_size,
        header_crc32,
        current_lba: le_u64(&bytes[24..32]),
        backup_lba: le_u64(&bytes[32..40]),
        first_usable_lba: le_u64(&bytes[40..48]),
        last_usable_lba: le_u64(&bytes[48..56]),
        partition_entries_lba: le_u64(&bytes[72..80]),
        partition_entry_count,
        partition_entry_size,
        partition_entries_crc32: le_u32(&bytes[88..92]),
    })
}

fn table_bytes(header: &GptHeader) -> RecoveryResult<u64> {
    let bytes = u64::from(header.partition_entry_count)
        .checked_mul(u64::from(header.partition_entry_size))
        .ok_or(RecoveryError::RangeOverflow)?;
    if bytes > MAX_TABLE_BYTES {
        return Err(RecoveryError::LengthTooLarge { length: bytes });
    }
    Ok(bytes)
}

fn read_header_at<D: BlockDevice>(device: &D, geometry: DiskGeometry, lba: u64) -> RecoveryResult<GptHeader> {
    let offset = lba.checked_mul(geometry.logical_sector_size).ok_or(RecoveryError::RangeOverflow)?;
    let sector_len = usize::try_from(geometry.logical_sector_size)
        .map_err(|_| RecoveryError::LengthTooLarge { length: geometry.logical_sector_size })?;
    let mut sector = vec![0u8; sector_len];
    read_exact_at(device, offset, &mut sector)?;
    parse_gpt_header(&sector)
}

fn load_table<D: BlockDevice>(device: &D, geometry: DiskGeometry, header: &GptHeader) -> RecoveryResult<Vec<u8>> {
    let bytes = table_bytes(header)?;
    let offset = header.partition_entries_lba.checked_mul(geometry.logical_sector_size).ok_or(RecoveryError::RangeOverflow)?;
    let len = usize::try_from(bytes).map_err(|_| RecoveryError::LengthTooLarge { length: bytes })?;
    let mut table = vec![0u8; len];
    read_exact_at(device, offset, &mut table)?;
    if crc32(&table) != header.partition_entries_crc32 {
        return Err(RecoveryError::IoFailure("GPT partition entry array CRC mismatch".into()));
    }
    Ok(table)
}

fn parse_table<D: BlockDevice>(device: &D, geometry: DiskGeometry, header: &GptHeader, table: &[u8]) -> RecoveryResult<DiscoveryResult> {
    if header.current_lba >= header.backup_lba || header.first_usable_lba > header.last_usable_lba {
        return Err(RecoveryError::IoFailure("invalid GPT header geometry".into()));
    }
    let mut result = DiscoveryResult::default();
    let entry_size = usize::try_from(header.partition_entry_size).map_err(|_| RecoveryError::LengthTooLarge { length: u64::from(header.partition_entry_size) })?;
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

pub fn discover_gpt<D: BlockDevice>(device: &D, geometry: DiskGeometry) -> RecoveryResult<DiscoveryResult> {
    let primary = match read_header_at(device, geometry, 1) {
        Ok(header) => header,
        Err(primary_error) => {
            let sectors = device.capacity() / geometry.logical_sector_size;
            if sectors < 2 { return Err(primary_error); }
            read_header_at(device, geometry, sectors - 1)?
        }
    };
    let table = load_table(device, geometry, &primary)?;
    parse_table(device, geometry, &primary, &table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes() -> Vec<u8> {
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
        let crc = crc32(&b[..92]);
        b[16..20].copy_from_slice(&crc.to_le_bytes());
        b
    }

    #[test]
    fn parses_crc_valid_header() {
        let b = header_bytes();
        assert_eq!(parse_gpt_header(&b).unwrap().partition_entry_count, 4);
    }

    #[test]
    fn rejects_crc_mismatch() {
        let mut b = header_bytes();
        b[24] ^= 1;
        assert!(parse_gpt_header(&b).is_err());
    }

    #[test]
    fn rejects_short_header() { assert!(parse_gpt_header(&[0u8; 16]).is_err()); }
}
