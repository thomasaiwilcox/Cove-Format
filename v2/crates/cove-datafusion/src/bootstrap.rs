//! Footer and dataset bootstrap helpers for COVE-backed DataFusion datasets.

#[cfg(feature = "covi")]
mod covi;
#[cfg(feature = "covm")]
mod covm;
mod local;
mod overlay;
mod parse;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use cove_core::header::HEADER_SIZE;

use crate::{dataset_state::DatasetState, options::CoveTableSelection};

#[cfg(feature = "covm")]
pub use covm::{
    bootstrap_covm_local_file_with_options, bootstrap_covm_local_file_with_options_async,
};
#[cfg(feature = "covi")]
pub use local::bootstrap_bytes_with_covi_artifacts;
pub use local::{
    bootstrap_bytes, bootstrap_bytes_with_options, bootstrap_local_file,
    bootstrap_local_file_async, bootstrap_local_file_with_options,
    bootstrap_local_file_with_options_async, bootstrap_range_reader_with_options,
};
pub use overlay::{
    bootstrap_overlay_snapshot_with_options, bootstrap_overlay_snapshot_with_options_async,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoveMetadataCacheKey {
    pub source: Arc<str>,
    pub file_id: [u8; 16],
    pub header_bytes: [u8; HEADER_SIZE],
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub table_selection: Option<CoveTableSelection>,
    pub options_fingerprint: u64,
}

#[derive(Debug, Default)]
pub struct CoveMetadataCache {
    entries: Mutex<HashMap<CoveMetadataCacheKey, Arc<DatasetState>>>,
}

impl CoveMetadataCache {
    fn entries(&self) -> MutexGuard<'_, HashMap<CoveMetadataCacheKey, Arc<DatasetState>>> {
        match self.entries.lock() {
            Ok(entries) => entries,
            // INVARIANT: cache poisoning must not silently disable metadata reuse.
            // The cache only stores immutable DatasetState values, so recovering
            // the guard is deterministic and keeps fallback behavior visible in
            // tests instead of degrading to repeated reparsing.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn get(&self, key: &CoveMetadataCacheKey) -> Option<Arc<DatasetState>> {
        self.entries().get(key).cloned()
    }

    pub fn insert(&self, key: CoveMetadataCacheKey, state: Arc<DatasetState>) {
        self.entries().insert(key, state);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ops::Range,
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use super::{bootstrap_range_reader_with_options, CoveMetadataCache, CoveMetadataCacheKey};
    use crate::{
        options::CoveTableOptions,
        range_reader::{CoveRangeReader, MemoryRangeReader, RangeReadKind},
    };
    use async_trait::async_trait;
    use cove_core::{
        constants::{CoveLogicalType, CovePhysicalKind},
        header::{CoveHeaderV1, HEADER_SIZE},
        table::{ColumnEntry, TableCatalog, TableEntry},
        writer::ScanProfileCoveWriter,
        CoveError,
    };

    #[derive(Debug)]
    struct CountingRangeReader {
        inner: MemoryRangeReader,
        file_len: u64,
        full_metadata_reads: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CoveRangeReader for CountingRangeReader {
        async fn read_ranges(
            &self,
            ranges: &[Range<u64>],
            kind: RangeReadKind,
        ) -> Result<Vec<Vec<u8>>, CoveError> {
            if kind == RangeReadKind::Metadata
                && ranges
                    .iter()
                    .any(|range| range.start == 0 && range.end == self.file_len)
            {
                self.full_metadata_reads.fetch_add(1, Ordering::Relaxed);
            }
            self.inner.read_ranges(ranges, kind).await
        }
    }

    #[test]
    fn metadata_cache_reuses_bootstrapped_state() {
        let bytes = cache_test_bytes();
        let reader = MemoryRangeReader::new(bytes.clone());
        let cache = CoveMetadataCache::default();

        let first = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-hit",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();
        let second = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-hit",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn metadata_bootstrap_validates_full_file_once_before_caching() {
        let bytes = cache_test_bytes();
        let full_metadata_reads = Arc::new(AtomicUsize::new(0));
        let reader = CountingRangeReader {
            inner: MemoryRangeReader::new(bytes.clone()),
            file_len: bytes.len() as u64,
            full_metadata_reads: Arc::clone(&full_metadata_reads),
        };
        let cache = CoveMetadataCache::default();

        let first = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-hit-read-count",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();
        let second = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-hit-read-count",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(full_metadata_reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn metadata_bootstrap_rejects_filecode_column_without_dictionary() {
        let bytes =
            include_bytes!("../../../conformance/reject/cove_t_filecode_missing_dictionary.cove")
                .to_vec();
        let reader = MemoryRangeReader::new(bytes.clone());
        let result = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://missing-dictionary",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            None,
        ));

        assert!(matches!(result, Err(CoveError::BadFileCode)));
    }

    #[test]
    fn metadata_cache_key_separates_option_dependent_state() {
        let bytes = cache_test_bytes();
        let reader = MemoryRangeReader::new(bytes.clone());
        let cache = CoveMetadataCache::default();

        let first = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-options",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();
        let second = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-options",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default().with_target_morsels_per_partition(7),
            Some(&cache),
        ))
        .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert_ne!(
            first.target_morsels_per_partition(),
            second.target_morsels_per_partition()
        );
    }

    #[test]
    fn metadata_cache_key_separates_header_scoped_metadata_state() {
        let bytes = cache_test_bytes();
        let reader = MemoryRangeReader::new(bytes.clone());
        let cache = CoveMetadataCache::default();

        let first = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-header-state",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();

        let mut changed = bytes.clone();
        let mut header = CoveHeaderV1::parse(&changed[..HEADER_SIZE]).unwrap();
        header.profile_capability_section_id = 999;
        header.fast_metadata_section_id = 998;
        changed[..HEADER_SIZE].copy_from_slice(&header.serialize());
        let changed_reader = MemoryRangeReader::new(changed.clone());
        let second = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-header-state",
            changed.len() as u64,
            &changed_reader,
            CoveTableOptions::default(),
            Some(&cache),
        ))
        .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn metadata_cache_recovers_after_poison() {
        let bytes = cache_test_bytes();
        let reader = MemoryRangeReader::new(bytes.clone());
        let cache = CoveMetadataCache::default();
        let state = futures::executor::block_on(bootstrap_range_reader_with_options(
            "memory://cache-poison",
            bytes.len() as u64,
            &reader,
            CoveTableOptions::default(),
            None,
        ))
        .unwrap();
        let key = CoveMetadataCacheKey {
            source: Arc::from("memory://cache-poison"),
            file_id: *state.file_id(),
            header_bytes: CoveHeaderV1::parse(&bytes[..HEADER_SIZE])
                .unwrap()
                .serialize(),
            file_len: state.file_len(),
            footer_crc32c: state.footer_crc32c(),
            table_selection: None,
            options_fingerprint: CoveTableOptions::default().cache_fingerprint(),
        };

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = cache.entries.lock().unwrap();
            panic!("poison cache lock for recovery test");
        }));
        assert!(cache.entries.is_poisoned());

        cache.insert(key.clone(), Arc::clone(&state));
        let cached = cache.get(&key).unwrap();
        assert!(Arc::ptr_eq(&cached, &state));
    }

    fn cache_test_bytes() -> Vec<u8> {
        let catalog = TableCatalog {
            flags: 0,
            tables: vec![TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 0,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![ColumnEntry {
                    column_id: 1,
                    name: "id".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                }],
            }],
        };
        ScanProfileCoveWriter::new(catalog).write().unwrap()
    }
}
