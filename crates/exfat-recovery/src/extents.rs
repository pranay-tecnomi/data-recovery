//! Stream extents and recovery candidates for exFAT.
//!
//! exFAT describes allocation two ways. When the stream's NoFatChain flag is
//! set the run is contiguous and the FAT holds nothing for it, so extents come
//! from geometry. Otherwise the FAT chain is followed, with the same loop and
//! bounds protection used elsewhere.

use recovery_core::{ByteRange, Extent, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{
    bitmap::AllocationBitmap,
    boot::{io_error, ExfatVolume},
    directory::{cluster_chain, DirectoryEntry, ATTR_DIRECTORY},
};

/// How much of a stream the extents are believed to represent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtentState {
    Recoverable,
    PartiallyRecoverable,
    MetadataOnly,
}

/// Evidence class, mirroring the confidence specification. Derived from
/// evidence, never asserted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Unknown,
}

/// Extents for one stream, with the evidence behind them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamExtents {
    pub extents: Vec<Extent>,
    pub state: ExtentState,
    /// Length recorded in the stream entry. Untrusted.
    pub declared_size: u64,
    pub recovered_size: u64,
    pub diagnostics: Vec<String>,
}

/// Resolves an active file's stream into logically ordered extents.
pub fn stream_extents<D: BlockDevice>(
    device: &D,
    volume: &ExfatVolume,
    volume_range: ByteRange,
    entry: &DirectoryEntry,
) -> RecoveryResult<StreamExtents> {
    if entry.attributes & ATTR_DIRECTORY != 0 {
        return Err(io_error("cannot resolve stream extents for a directory"));
    }
    let declared_size = entry.data_length;
    let mut diagnostics = Vec::new();

    // valid_data_length records how much has actually been written; the tail
    // between it and data_length is uninitialised, not recovered content.
    if entry.valid_data_length < declared_size {
        diagnostics.push(format!(
            "only {} of {declared_size} bytes were ever written",
            entry.valid_data_length
        ));
    }

    if declared_size == 0 || !volume.is_valid_cluster(entry.first_cluster) {
        if declared_size > 0 {
            diagnostics.push("stream declares content but has no valid start cluster".into());
        }
        return Ok(StreamExtents {
            extents: Vec::new(),
            state: if declared_size == 0 { ExtentState::Recoverable } else { ExtentState::MetadataOnly },
            declared_size,
            recovered_size: 0,
            diagnostics,
        });
    }

    let cluster_size = volume.cluster_size()?;
    let needed = declared_size.div_ceil(cluster_size);

    let clusters = if entry.no_fat_chain {
        // Contiguous by declaration: the FAT holds no chain for this stream.
        contiguous_run(volume, entry.first_cluster, needed, &mut diagnostics)?
    } else {
        match cluster_chain(device, volume, volume_range, entry.first_cluster) {
            Ok(chain) => chain,
            Err(error) => {
                diagnostics.push(format!("cluster chain did not resolve cleanly: {error:?}"));
                partial_chain(device, volume, volume_range, entry.first_cluster)
            }
        }
    };

    let (extents, recovered_size) =
        build_extents(volume, volume_range, &clusters, declared_size, &mut diagnostics)?;

    let state = if declared_size == 0 {
        ExtentState::Recoverable
    } else if extents.is_empty() {
        ExtentState::MetadataOnly
    } else if recovered_size < declared_size {
        diagnostics.push(format!(
            "allocation supplied {recovered_size} of {declared_size} declared bytes"
        ));
        ExtentState::PartiallyRecoverable
    } else {
        ExtentState::Recoverable
    };

    Ok(StreamExtents { extents, state, declared_size, recovered_size, diagnostics })
}

/// Clusters of a contiguous run, stopping at the end of the heap.
fn contiguous_run(
    volume: &ExfatVolume,
    first: u32,
    needed: u64,
    diagnostics: &mut Vec<String>,
) -> RecoveryResult<Vec<u32>> {
    let mut clusters = Vec::new();
    for index in 0..needed {
        let cluster = u64::from(first)
            .checked_add(index)
            .ok_or(RecoveryError::RangeOverflow)?;
        let cluster = u32::try_from(cluster).map_err(|_| io_error("cluster number overflow"))?;
        if !volume.is_valid_cluster(cluster) {
            diagnostics.push("contiguous run reaches the end of the cluster heap".into());
            break;
        }
        clusters.push(cluster);
    }
    Ok(clusters)
}

/// Resolves as much of a chain as remains valid, keeping the surviving prefix
/// so a damaged tail cannot discard a recoverable head.
fn partial_chain<D: BlockDevice>(
    device: &D,
    volume: &ExfatVolume,
    volume_range: ByteRange,
    start: u32,
) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let bound = usize::try_from(volume.cluster_count).unwrap_or(usize::MAX);
    let mut current = start;
    while chain.len() <= bound {
        if !volume.is_valid_cluster(current) || !seen.insert(current) {
            break;
        }
        chain.push(current);
        match crate::directory::next_cluster(device, volume, volume_range, current) {
            Ok(Some(next)) => current = next,
            Ok(None) | Err(_) => break,
        }
    }
    chain
}

/// Converts clusters into logically ordered extents, trimming the final
/// cluster to the declared size and coalescing adjacent runs.
fn build_extents(
    volume: &ExfatVolume,
    volume_range: ByteRange,
    clusters: &[u32],
    declared_size: u64,
    diagnostics: &mut Vec<String>,
) -> RecoveryResult<(Vec<Extent>, u64)> {
    let cluster_size = volume.cluster_size()?;
    let volume_limit = volume_range.end()?;
    let mut extents: Vec<Extent> = Vec::new();
    let mut logical: u64 = 0;

    for &cluster in clusters {
        // Stop once the declared size is covered; the rest is cluster slack.
        if logical >= declared_size {
            break;
        }
        let offset = volume.cluster_offset(volume_range.offset, cluster)?;
        let length = (declared_size - logical).min(cluster_size);
        let range = ByteRange::new(offset, length)?;
        if range.end()? > volume_limit {
            diagnostics.push(format!("cluster {cluster} extends outside the volume"));
            break;
        }
        match extents.last_mut() {
            Some(last) if last.source_range.end()? == offset => {
                last.source_range =
                    ByteRange::new(last.source_range.offset, last.source_range.length + length)?;
            }
            _ => extents.push(Extent::new(range, logical)?),
        }
        logical = logical
            .checked_add(length)
            .ok_or(RecoveryError::RangeOverflow)?;
    }

    let recovered = recovery_core::total_length(&extents)?;
    recovery_core::validate_logical_layout(&extents)?;
    Ok((extents, recovered))
}

/// A deleted exFAT file candidate with the evidence behind its classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeletedCandidate {
    /// exFAT keeps the full name in its File Name entries, so unlike FAT32 no
    /// character is lost to a deletion tombstone.
    pub name: String,
    pub first_cluster: u32,
    pub declared_size: u64,
    pub extents: Vec<Extent>,
    pub state: ExtentState,
    pub confidence: Confidence,
    /// Append-only reasons supporting the classification.
    pub evidence: Vec<String>,
}

/// Reconstructs a candidate from a deleted entry set.
///
/// The allocation bitmap is evidence, not proof: a cluster marked free may
/// still have been overwritten. A cluster marked in use has been handed to
/// another file, so claiming it would fabricate content.
pub fn deleted_candidate(
    volume: &ExfatVolume,
    volume_range: ByteRange,
    bitmap: Option<&AllocationBitmap>,
    entry: &DirectoryEntry,
) -> RecoveryResult<DeletedCandidate> {
    if !entry.deleted {
        return Err(io_error("entry set is not deleted"));
    }
    let declared_size = entry.data_length;
    let mut evidence = Vec::new();
    evidence.push("name recovered intact from the file name entries".into());

    if declared_size == 0 || !volume.is_valid_cluster(entry.first_cluster) {
        evidence.push("entry set survives but no content is locatable".into());
        return Ok(DeletedCandidate {
            name: entry.name.clone(),
            first_cluster: entry.first_cluster,
            declared_size,
            extents: Vec::new(),
            state: ExtentState::MetadataOnly,
            confidence: Confidence::Unknown,
            evidence,
        });
    }

    let cluster_size = volume.cluster_size()?;
    let needed = declared_size.div_ceil(cluster_size);
    let mut diagnostics = Vec::new();
    let mut clusters = Vec::new();
    let mut reallocated = false;

    for index in 0..needed {
        let cluster = u64::from(entry.first_cluster)
            .checked_add(index)
            .ok_or(RecoveryError::RangeOverflow)?;
        let cluster = u32::try_from(cluster).map_err(|_| io_error("cluster number overflow"))?;
        if !volume.is_valid_cluster(cluster) {
            evidence.push("inferred run reaches the end of the cluster heap".into());
            break;
        }
        // A cluster back in use holds another file's data.
        if let Some(bitmap) = bitmap
            && bitmap.is_allocated(cluster) == Some(true)
        {
            evidence.push(format!(
                "cluster {cluster} is allocated to another file; content is overwritten"
            ));
            reallocated = true;
            break;
        }
        clusters.push(cluster);
    }

    let (extents, recovered_size) =
        build_extents(volume, volume_range, &clusters, declared_size, &mut diagnostics)?;
    evidence.extend(diagnostics);

    let state = if extents.is_empty() {
        ExtentState::MetadataOnly
    } else if recovered_size < declared_size {
        ExtentState::PartiallyRecoverable
    } else {
        ExtentState::Recoverable
    };

    // A deleted entry set's NoFatChain flag records that the stream *was*
    // contiguous, which is stronger evidence than FAT32 leaves behind. It is
    // still not proof the content survived untouched.
    if entry.no_fat_chain {
        evidence.push("stream was recorded as contiguous, so the run is not inferred".into());
    } else {
        evidence.push(
            "cluster chain released by deletion; contiguous allocation inferred".into(),
        );
        if needed > 1 {
            evidence.push("multi-cluster file may have been fragmented; contiguity is unverified".into());
        }
    }
    if bitmap.is_none() {
        evidence.push("allocation bitmap unavailable; overwrite could not be checked".into());
    }

    let confidence = match state {
        ExtentState::MetadataOnly => Confidence::Unknown,
        ExtentState::PartiallyRecoverable => Confidence::Low,
        ExtentState::Recoverable => {
            // Contiguity that was recorded on disk, or a single-cluster file
            // that cannot be fragmented, beats a bare inference. Either still
            // needs the bitmap to show the clusters were never reused.
            let contiguity_is_evidenced = entry.no_fat_chain || needed == 1;
            if contiguity_is_evidenced && bitmap.is_some() && !reallocated {
                Confidence::Medium
            } else {
                Confidence::Low
            }
        }
    };

    Ok(DeletedCandidate {
        name: entry.name.clone(),
        first_cluster: entry.first_cluster,
        declared_size,
        extents,
        state,
        confidence,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimage::{image, Mem, BITMAP_CLUSTER, CLUSTER, CLUSTER_COUNT, HEAP_SECTOR, SECTOR};

    fn volume(m: &Mem) -> (ExfatVolume, ByteRange) {
        (crate::parse_volume(m, m.range()).unwrap(), m.range())
    }

    fn entry(first_cluster: u32, size: u64, no_fat_chain: bool, deleted: bool) -> DirectoryEntry {
        DirectoryEntry {
            name: "File.bin".into(),
            attributes: 0,
            first_cluster,
            data_length: size,
            valid_data_length: size,
            no_fat_chain,
            deleted,
        }
    }

    fn bitmap(m: &Mem) -> AllocationBitmap {
        let (v, r) = volume(m);
        AllocationBitmap::load(m, &v, r, BITMAP_CLUSTER, u64::from(CLUSTER_COUNT).div_ceil(8)).unwrap()
    }

    fn cluster_offset(n: u32) -> u64 {
        (HEAP_SECTOR * SECTOR + (n as usize - 2) * CLUSTER) as u64
    }

    #[test]
    fn contiguous_stream_needs_no_fat_chain() {
        let m = image();
        let (v, r) = volume(&m);
        // No FAT links are set for clusters 5..7 at all.
        let s = stream_extents(&m, &v, r, &entry(5, 1500, true, false)).unwrap();
        assert_eq!(s.state, ExtentState::Recoverable);
        // Three adjacent clusters coalesce into a single extent.
        assert_eq!(s.extents.len(), 1);
        assert_eq!(s.extents[0].source_range.offset, cluster_offset(5));
        assert_eq!(s.recovered_size, 1500);
    }

    #[test]
    fn fragmented_stream_follows_the_fat_chain() {
        let mut m = image();
        m.link(5, 9);
        m.link(9, 0xFFFF_FFFF);
        let (v, r) = volume(&m);
        let s = stream_extents(&m, &v, r, &entry(5, 1000, false, false)).unwrap();
        assert_eq!(s.state, ExtentState::Recoverable);
        // A physical gap between clusters 5 and 9 forces two extents.
        assert_eq!(s.extents.len(), 2);
        assert_eq!(s.extents[0].logical_offset, 0);
        assert_eq!(s.extents[1].logical_offset, 512);
        assert_eq!(s.extents[1].source_range.offset, cluster_offset(9));
        assert!(recovery_core::validate_logical_layout(&s.extents).is_ok());
    }

    #[test]
    fn final_cluster_is_trimmed_to_the_declared_size() {
        let mut m = image();
        m.link(5, 6);
        m.link(6, 0xFFFF_FFFF);
        let (v, r) = volume(&m);
        let s = stream_extents(&m, &v, r, &entry(5, 600, false, false)).unwrap();
        // 600 bytes spans two clusters but must not include slack.
        assert_eq!(s.recovered_size, 600);
    }

    #[test]
    fn chain_shorter_than_declared_size_is_partial() {
        let mut m = image();
        m.link(5, 0xFFFF_FFFF);
        let (v, r) = volume(&m);
        let s = stream_extents(&m, &v, r, &entry(5, 4096, false, false)).unwrap();
        assert_eq!(s.state, ExtentState::PartiallyRecoverable);
        assert_eq!(s.recovered_size, 512);
        assert!(!s.diagnostics.is_empty());
    }

    #[test]
    fn broken_chain_keeps_the_recoverable_prefix() {
        let mut m = image();
        m.link(5, 6);
        m.link(6, 9999);
        let (v, r) = volume(&m);
        let s = stream_extents(&m, &v, r, &entry(5, 4096, false, false)).unwrap();
        assert_eq!(s.state, ExtentState::PartiallyRecoverable);
        // A damaged tail must not discard the valid head.
        assert_eq!(s.recovered_size, 1024);
        assert!(s.diagnostics.iter().any(|d| d.contains("did not resolve cleanly")));
    }

    #[test]
    fn unwritten_tail_is_reported() {
        let m = image();
        let (v, r) = volume(&m);
        let mut e = entry(5, 2000, true, false);
        // Only 500 bytes were ever written; the rest is uninitialised.
        e.valid_data_length = 500;
        let s = stream_extents(&m, &v, r, &e).unwrap();
        assert!(s.diagnostics.iter().any(|d| d.contains("were ever written")));
    }

    #[test]
    fn contiguous_run_stops_at_the_end_of_the_heap() {
        let m = image();
        let (v, r) = volume(&m);
        // Start near the last cluster and declare far more than remains.
        let s = stream_extents(&m, &v, r, &entry(CLUSTER_COUNT, 1 << 20, true, false)).unwrap();
        for e in &s.extents {
            assert!(e.source_range.end().unwrap() <= r.end().unwrap());
        }
        assert_eq!(s.state, ExtentState::PartiallyRecoverable);
    }

    #[test]
    fn zero_length_stream_yields_no_extents() {
        let m = image();
        let (v, r) = volume(&m);
        let s = stream_extents(&m, &v, r, &entry(5, 0, true, false)).unwrap();
        assert!(s.extents.is_empty());
        assert_eq!(s.state, ExtentState::Recoverable);
    }

    #[test]
    fn stream_without_a_valid_start_cluster_is_metadata_only() {
        let m = image();
        let (v, r) = volume(&m);
        let s = stream_extents(&m, &v, r, &entry(0, 500, true, false)).unwrap();
        assert_eq!(s.state, ExtentState::MetadataOnly);
    }

    #[test]
    fn rejects_a_directory_entry() {
        let m = image();
        let (v, r) = volume(&m);
        let mut d = entry(5, 0, true, false);
        d.attributes = ATTR_DIRECTORY;
        assert!(stream_extents(&m, &v, r, &d).is_err());
    }

    #[test]
    fn deleted_contiguous_file_reaches_medium_confidence() {
        let m = image();
        let (v, r) = volume(&m);
        let b = bitmap(&m);
        // Contiguity recorded on disk, and the clusters are still free.
        let c = deleted_candidate(&v, r, Some(&b), &entry(5, 1500, true, true)).unwrap();
        assert_eq!(c.state, ExtentState::Recoverable);
        assert_eq!(c.confidence, Confidence::Medium);
        assert!(c.evidence.iter().any(|e| e.contains("recorded as contiguous")));
    }

    #[test]
    fn deleted_fragmented_multi_cluster_file_stays_low() {
        let m = image();
        let (v, r) = volume(&m);
        let b = bitmap(&m);
        // No contiguity record: the run is inferred across several clusters.
        let c = deleted_candidate(&v, r, Some(&b), &entry(5, 1500, false, true)).unwrap();
        assert_eq!(c.confidence, Confidence::Low);
        assert!(c.evidence.iter().any(|e| e.contains("fragmented")));
    }

    #[test]
    fn reallocated_cluster_truncates_the_candidate() {
        let mut m = image();
        // Cluster 6 has been handed to another file.
        m.allocate(6);
        let (v, r) = volume(&m);
        let b = bitmap(&m);
        let c = deleted_candidate(&v, r, Some(&b), &entry(5, 1500, true, true)).unwrap();
        assert_eq!(c.state, ExtentState::PartiallyRecoverable);
        assert_eq!(c.confidence, Confidence::Low);
        assert_eq!(c.extents[0].source_range.length, 512);
        assert!(c.evidence.iter().any(|e| e.contains("overwritten")));
    }

    #[test]
    fn fully_reallocated_file_yields_no_extents() {
        let mut m = image();
        m.allocate(5);
        let (v, r) = volume(&m);
        let b = bitmap(&m);
        let c = deleted_candidate(&v, r, Some(&b), &entry(5, 512, true, true)).unwrap();
        assert_eq!(c.state, ExtentState::MetadataOnly);
        assert_eq!(c.confidence, Confidence::Unknown);
    }

    #[test]
    fn deleted_name_survives_intact() {
        let m = image();
        let (v, r) = volume(&m);
        let b = bitmap(&m);
        let mut e = entry(5, 100, true, true);
        e.name = "Quarterly Report.pdf".into();
        let c = deleted_candidate(&v, r, Some(&b), &e).unwrap();
        // Unlike FAT32, no character is lost to a deletion tombstone.
        assert_eq!(c.name, "Quarterly Report.pdf");
        assert!(c.evidence.iter().any(|x| x.contains("intact")));
    }

    #[test]
    fn missing_bitmap_lowers_confidence_and_is_recorded() {
        let m = image();
        let (v, r) = volume(&m);
        // Without the bitmap, overwrite cannot be ruled out.
        let c = deleted_candidate(&v, r, None, &entry(5, 512, true, true)).unwrap();
        assert_eq!(c.confidence, Confidence::Low);
        assert!(c.evidence.iter().any(|e| e.contains("bitmap unavailable")));
    }

    #[test]
    fn rejects_an_active_entry() {
        let m = image();
        let (v, r) = volume(&m);
        assert!(deleted_candidate(&v, r, None, &entry(5, 100, true, false)).is_err());
    }

    #[test]
    fn no_candidate_is_ever_high_confidence() {
        let m = image();
        let (v, r) = volume(&m);
        let b = bitmap(&m);
        for e in [
            entry(5, 100, true, true),
            entry(5, 5000, false, true),
            entry(0, 10, true, true),
            entry(5, 512, true, true),
        ] {
            let c = deleted_candidate(&v, r, Some(&b), &e).unwrap();
            // Deleted metadata is weaker evidence; content validation runs later.
            assert_ne!(c.confidence, Confidence::High);
            assert!(!c.evidence.is_empty());
        }
    }
}
