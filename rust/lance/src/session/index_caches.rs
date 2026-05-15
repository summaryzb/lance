// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Lance Authors

//! Caches for Lance indices. They are organized in a hierarchical manner to
//! avoid collisions.
//!
//!  GlobalIndexCache
//!     │
//!     ├─► DSIndexCache (prefixed by dataset URI)
//!     │    │
//!     └────┴──► Index-specific cache (prefixed by index UUID and FRI UUID)
//!
//! Cached value types include scalar/vector index instances, IVF partitions,
//! posting lists, bitmaps, and zonemap segment batches. Zonemap segments are
//! keyed by their per-segment IndexMetadata UUID, which is regenerated on
//! every write — so consolidation/compaction naturally produces fresh keys
//! and old entries age out via LRU.

use std::{borrow::Cow, ops::Deref, sync::Arc};

use arrow_array::RecordBatch;
use deepsize::{Context, DeepSizeOf};
use lance_core::cache::{CacheKey, LanceCache};
use lance_index::frag_reuse::FragReuseIndex;
use lance_table::format::IndexMetadata;
use uuid::Uuid;

/// A type-safe wrapper around a LanceCache that enforces namespaces for index data.
pub struct GlobalIndexCache(pub(super) LanceCache);

impl GlobalIndexCache {
    pub fn for_dataset(&self, uri: &str) -> DSIndexCache {
        // Create a sub-cache for the dataset by adding the URI as a key prefix.
        // This prevents collisions between different datasets.
        DSIndexCache(self.0.with_key_prefix(uri))
    }
}

impl Clone for GlobalIndexCache {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Deref for GlobalIndexCache {
    type Target = LanceCache;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DeepSizeOf for GlobalIndexCache {
    fn deep_size_of_children(&self, context: &mut Context) -> usize {
        self.0.deep_size_of_children(context)
    }
}

/// A type-safe wrapper around a LanceCache that enforces namespaces and keys
/// for dataset-specific index data.
pub struct DSIndexCache(pub(crate) LanceCache);

impl Deref for DSIndexCache {
    type Target = LanceCache;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DSIndexCache {
    /// Create an index-specific cache with the given UUID prefix.
    pub fn for_index(&self, uuid: &str, fri_uuid: Option<&Uuid>) -> LanceCache {
        if let Some(fri_uuid) = fri_uuid {
            // If a FRI UUID is provided, use it to create a more specific cache key.
            let cache_key = format!("{}-{}", uuid, fri_uuid);
            self.0.with_key_prefix(&cache_key)
        } else {
            // Otherwise, just use the index UUID as the key prefix.
            self.0.with_key_prefix(uuid)
        }
    }
}

// Cache key types for type-safe cache access

#[derive(Debug)]
pub struct FragReuseIndexKey<'a> {
    pub uuid: &'a str,
}

impl CacheKey for FragReuseIndexKey<'_> {
    type ValueType = FragReuseIndex;

    fn key(&self) -> Cow<'_, str> {
        Cow::Owned(format!("frag_reuse/{}", self.uuid))
    }

    fn type_name() -> &'static str {
        "FragReuseIndex"
    }
}

#[derive(Debug)]
pub struct IndexMetadataKey {
    pub version: u64,
}

impl CacheKey for IndexMetadataKey {
    type ValueType = Vec<IndexMetadata>;

    fn key(&self) -> Cow<'_, str> {
        Cow::Owned(self.version.to_string())
    }

    fn type_name() -> &'static str {
        "Vec<IndexMetadata>"
    }

    fn codec() -> Option<lance_core::cache::CacheCodec> {
        Some(lance_table::format::index_metadata_codec())
    }
}

pub struct ProstAny(pub Arc<prost_types::Any>);

impl DeepSizeOf for ProstAny {
    fn deep_size_of_children(&self, context: &mut Context) -> usize {
        self.0.type_url.deep_size_of_children(context) + self.0.value.deep_size_of_children(context)
    }
}

/// Cache key for scalar index details
///
/// Typically we don't use the cache for scalar index details because they are stored
/// in the manifest and readily available.  However, old versions of Lance didn't store
/// details in the manifest, and we have to perform an expensive inference process to determine
/// what they are.  These we cache.
#[derive(Debug)]
pub struct ScalarIndexDetailsKey<'a> {
    pub uuid: &'a str,
}

impl CacheKey for ScalarIndexDetailsKey<'_> {
    type ValueType = ProstAny;

    fn key(&self) -> Cow<'_, str> {
        Cow::Owned(format!("type/{}", self.uuid))
    }

    fn type_name() -> &'static str {
        "ScalarIndexDetails"
    }
}

/// Newtype for cached zonemap segment batches.
///
/// Stored under one IndexMetadata UUID per segment. The cache returns this
/// wrapped in `Arc` (the `LanceCache` API does the wrap, see
/// `rust/lance-core/src/cache/mod.rs:308-326`), so we deliberately do **not**
/// add an inner `Arc<...>`: that would only buy us a second allocation with
/// no extra sharing capability.
pub struct ZoneMapStatsBatches(pub Vec<RecordBatch>);

impl DeepSizeOf for ZoneMapStatsBatches {
    fn deep_size_of_children(&self, _context: &mut Context) -> usize {
        // RecordBatch buffers are Arc-shared. `get_array_memory_size` returns
        // the (possibly over-counted) full backing-buffer size, which is the
        // safer direction for an LRU byte budget — over-eviction is a soft
        // failure, under-eviction can OOM. Sum across all batches in this
        // segment.
        self.0.iter().map(|b| b.get_array_memory_size()).sum()
    }
}

/// Cache key for one zonemap index segment, identified by its IndexMetadata UUID.
///
/// Scoped under `DSIndexCache` (dataset URI prefix), so the same UUID across
/// different datasets does not collide.
#[derive(Debug)]
pub struct ZoneMapStatsKey<'a> {
    pub uuid: &'a str,
}

impl CacheKey for ZoneMapStatsKey<'_> {
    type ValueType = ZoneMapStatsBatches;

    fn key(&self) -> Cow<'_, str> {
        Cow::Owned(format!("zonemap_stats/{}", self.uuid))
    }

    fn type_name() -> &'static str {
        "ZoneMapStatsBatches"
    }
}

#[cfg(test)]
mod zonemap_key_tests {
    use super::*;
    use lance_core::cache::CacheKey;

    #[test]
    fn zonemap_stats_key_contains_uuid_and_namespace() {
        let key = ZoneMapStatsKey { uuid: "abcd-1234" };
        let s = key.key().into_owned();
        assert!(s.starts_with("zonemap_stats/"), "got: {}", s);
        assert!(s.contains("abcd-1234"), "got: {}", s);
    }

    #[test]
    fn zonemap_stats_key_distinguishes_different_uuids() {
        let a = ZoneMapStatsKey { uuid: "u1" }.key().into_owned();
        let b = ZoneMapStatsKey { uuid: "u2" }.key().into_owned();
        assert_ne!(a, b);
    }

    #[allow(dead_code)]
    fn _assert_zonemap_stats_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ZoneMapStatsBatches>();
    }
}
