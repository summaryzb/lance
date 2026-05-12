// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Lance Authors

use std::collections::HashMap;
use std::sync::Arc;

use hdfs_native_object_store::HdfsObjectStoreBuilder;
use log::debug;
use url::Url;

use crate::object_store::{
    DEFAULT_MAX_IOP_SIZE, ObjectStore, ObjectStoreParams, ObjectStoreProvider, StorageOptions,
};
use lance_core::error::{Error, Result};

// HDFS is typically colocated or low-latency; cloud defaults (64 KiB block,
// 64-way parallelism) amplify namenode/datanode RPC without benefit.
const DEFAULT_HDFS_BLOCK_SIZE: usize = 1024 * 1024;
const DEFAULT_HDFS_IO_PARALLELISM: usize = 16;

#[derive(Default, Debug)]
pub struct HdfsStoreProvider;

#[async_trait::async_trait]
impl ObjectStoreProvider for HdfsStoreProvider {
    async fn new_store(&self, base_path: Url, params: &ObjectStoreParams) -> Result<ObjectStore> {
        // HDFS requires a nameservice authority — reject `hdfs:///path` early
        // with an actionable error rather than letting the builder report an
        // opaque failure.
        if base_path.host_str().is_none_or(str::is_empty) {
            return Err(Error::invalid_input(format!(
                "HDFS URL must include a nameservice authority, got '{}'",
                base_path,
            )));
        }

        let block_size = params.block_size.unwrap_or(DEFAULT_HDFS_BLOCK_SIZE);

        // Prefer the async accessor so namespace-driven dynamic credentials work;
        // fall back to static storage_options if no accessor is attached.
        let storage_options = if let Some(accessor) = params.get_accessor() {
            accessor.get_storage_options().await.unwrap_or_else(|_| {
                StorageOptions(params.storage_options().cloned().unwrap_or_default())
            })
        } else {
            StorageOptions(params.storage_options().cloned().unwrap_or_default())
        };

        // Builder wants the bare `hdfs://authority` URL; drop the path component
        // using the url crate instead of reassembling the string manually.
        let mut builder_url = base_path.clone();
        builder_url.set_path("");
        let builder_url_str = builder_url.as_str();

        // HDFS_* env vars -> Hadoop keys (underscore to dot).
        let mut config: HashMap<String, String> = std::env::vars()
            .filter(|(k, _)| k.starts_with("HDFS_"))
            .map(|(k, v)| {
                let key = k
                    .to_lowercase()
                    .strip_prefix("hdfs_")
                    .unwrap()
                    .replace('_', ".");
                (key, v)
            })
            .collect();

        // storage_options override env; pass through standard Hadoop key prefixes.
        for (k, v) in &storage_options.0 {
            if k.starts_with("fs.") || k.starts_with("dfs.") || k.starts_with("hadoop.") {
                config.insert(k.clone(), v.clone());
            }
        }

        let config_keys = config.keys().cloned().collect::<Vec<_>>().join(", ");
        debug!(
            "Creating HDFS store url={} keys=[{}]",
            builder_url_str, config_keys
        );

        let hdfs_store = HdfsObjectStoreBuilder::new()
            .with_url(builder_url_str)
            .with_config(config)
            .build()
            .map_err(|e| {
                Error::invalid_input(format!(
                    "Failed to create HDFS store for '{}': {:?}. Keys=[{}]. \
                     Ensure HA config (dfs.ha.namenodes.*, dfs.namenode.rpc-address.*) \
                     is set via storage_options or HADOOP_CONF_DIR.",
                    builder_url_str, e, config_keys,
                ))
            })?;

        let mut url = base_path;
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }

        Ok(ObjectStore {
            scheme: "hdfs".to_string(),
            inner: Arc::new(hdfs_store),
            block_size,
            max_iop_size: *DEFAULT_MAX_IOP_SIZE,
            use_constant_size_upload_parts: params.use_constant_size_upload_parts,
            list_is_lexically_ordered: params.list_is_lexically_ordered.unwrap_or(false),
            io_parallelism: DEFAULT_HDFS_IO_PARALLELISM,
            download_retry_count: storage_options.download_retry_count(),
            io_tracker: Default::default(),
            store_prefix: self.calculate_object_store_prefix(&url, params.storage_options())?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hdfs_store_path() {
        let provider = HdfsStoreProvider;
        let url = Url::parse("hdfs://mycluster/path/to/file").unwrap();
        let path = provider.extract_path(&url).unwrap();
        assert_eq!(path, object_store::path::Path::from("/path/to/file"));
    }

    #[test]
    fn test_hdfs_store_prefix() {
        let provider = HdfsStoreProvider;
        let url = Url::parse("hdfs://mycluster/some/path/").unwrap();
        let prefix = provider.calculate_object_store_prefix(&url, None).unwrap();
        assert_eq!(prefix, "hdfs$mycluster");
    }

    #[tokio::test]
    async fn test_hdfs_new_store_rejects_missing_authority() {
        let provider = HdfsStoreProvider;
        let url = Url::parse("hdfs:///path/to/file").unwrap();
        let err = provider
            .new_store(url, &ObjectStoreParams::default())
            .await
            .expect_err("hdfs:/// must be rejected");
        assert!(
            err.to_string().contains("nameservice authority"),
            "unexpected error: {err}"
        );
    }
}
