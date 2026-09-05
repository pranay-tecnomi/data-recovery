use recovery_core::{ByteRange, CandidateId, RecoveryError, RecoveryResult, SourceId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CandidateEvidence {
    ActiveDirectoryEntry,
    DeletedDirectoryEntry,
    CarvedSignature,
}

impl CandidateEvidence {
    fn base_score(self) -> u8 {
        match self {
            Self::ActiveDirectoryEntry => 90,
            Self::DeletedDirectoryEntry => 65,
            Self::CarvedSignature => 40,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryCandidate {
    pub id: CandidateId,
    pub source_id: SourceId,
    pub path: String,
    pub size: u64,
    pub extents: Vec<ByteRange>,
    pub evidence: CandidateEvidence,
    pub score: u8,
}

impl RecoveryCandidate {
    pub fn new(
        id: CandidateId,
        source_id: SourceId,
        path: impl Into<String>,
        size: u64,
        extents: Vec<ByteRange>,
        evidence: CandidateEvidence,
    ) -> RecoveryResult<Self> {
        let path = normalize_path(&path.into())?;
        let score = evidence.base_score();
        let candidate = Self { id, source_id, path, size, extents, evidence, score };
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn validate(&self) -> RecoveryResult<()> {
        if self.path.is_empty() || self.path == "." || self.path == ".." {
            return Err(RecoveryError::IoFailure("recovery candidate has an invalid path".into()));
        }
        let total = self.extents.iter().try_fold(0u64, |total, extent| {
            total.checked_add(extent.length).ok_or(RecoveryError::RangeOverflow)
        })?;
        if total != self.size {
            return Err(RecoveryError::IoFailure("candidate extent length does not match candidate size".into()));
        }
        for pair in self.extents.windows(2) {
            let left_end = pair[0].offset.checked_add(pair[0].length).ok_or(RecoveryError::RangeOverflow)?;
            if left_end > pair[1].offset {
                return Err(RecoveryError::IoFailure("candidate extents overlap".into()));
            }
        }
        Ok(())
    }

    pub fn dedup_key(&self) -> (SourceId, u64, Vec<ByteRange>) {
        (self.source_id.clone(), self.size, self.extents.clone())
    }
}

pub fn normalize_path(path: &str) -> RecoveryResult<String> {
    let mut components = Vec::new();
    for component in path.replace('\\', "/").split('/') {
        match component.trim() {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(RecoveryError::IoFailure("candidate path escapes its recovery root".into()));
                }
            }
            value => {
                if value.contains('\0') {
                    return Err(RecoveryError::IoFailure("candidate path contains NUL".into()));
                }
                components.push(value.to_string());
            }
        }
    }
    if components.is_empty() {
        return Err(RecoveryError::IoFailure("candidate path is empty".into()));
    }
    Ok(components.join("/"))
}

pub fn deduplicate(candidates: impl IntoIterator<Item = RecoveryCandidate>) -> Vec<RecoveryCandidate> {
    let mut output = Vec::new();
    for candidate in candidates {
        if let Some(existing) = output.iter_mut().find(|item: &&mut RecoveryCandidate| item.dedup_key() == candidate.dedup_key()) {
            if candidate.score > existing.score { *existing = candidate; }
        } else {
            output.push(candidate);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(offset: u64, length: u64) -> ByteRange { ByteRange::new(offset, length).unwrap() }

    #[test]
    fn normalizes_separators_and_dot_components() {
        assert_eq!(normalize_path(r"/Users//alice/./docs\\file.txt").unwrap(), "Users/alice/docs/file.txt");
    }

    #[test]
    fn rejects_root_escape() {
        assert!(normalize_path("../../file").is_err());
    }

    #[test]
    fn validates_extent_length_and_overlap() {
        let source = SourceId::new("disk-1");
        let valid = RecoveryCandidate::new(CandidateId::new("a"), source.clone(), "file", 20, vec![extent(100, 10), extent(200, 10)], CandidateEvidence::DeletedDirectoryEntry).unwrap();
        assert_eq!(valid.score, 65);
        assert!(RecoveryCandidate::new(CandidateId::new("b"), source.clone(), "bad", 15, vec![extent(100, 10), extent(105, 5)], CandidateEvidence::DeletedDirectoryEntry).is_err());
    }

    #[test]
    fn deduplicates_by_source_size_and_extents_and_keeps_stronger_evidence() {
        let source = SourceId::new("disk-1");
        let a = RecoveryCandidate::new(CandidateId::new("a"), source.clone(), "old.txt", 10, vec![extent(100, 10)], CandidateEvidence::DeletedDirectoryEntry).unwrap();
        let b = RecoveryCandidate::new(CandidateId::new("b"), source, "renamed.txt", 10, vec![extent(100, 10)], CandidateEvidence::ActiveDirectoryEntry).unwrap();
        let result = deduplicate([a, b]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].score, 90);
        assert_eq!(result[0].path, "renamed.txt");
    }
}
