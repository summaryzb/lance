// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Lance Authors

use std::collections::HashMap;
use std::sync::Arc;

use object_store_opendal::OpendalStore;
use opendal::layers::RetryLayer;
use opendal::{services::S3, Operator};
use url::Url;

use crate::object_store::{
    DEFAULT_CLOUD_BLOCK_SIZE, DEFAULT_CLOUD_IO_PARALLELISM, DEFAULT_MAX_IOP_SIZE, ObjectStore,
    ObjectStoreParams, ObjectStoreProvider, StorageOptions,
};
use lance_core::error::{Error, Result};

#[derive(Default, Debug)]
pub struct BosStoreProvider;

#[async_trait::async_trait]
impl ObjectStoreProvider for BosStoreProvider {
    async fn new_store(&self, base_path: Url, params: &ObjectStoreParams) -> Result<ObjectStore> {
        let block_size = params.block_size.unwrap_or(DEFAULT_CLOUD_BLOCK_SIZE);
        let storage_options = StorageOptions(params.storage_options().cloned().unwrap_or_default());

        let bucket = base_path
            .host_str()
            .ok_or_else(|| Error::invalid_input("BOS URL must contain bucket name"))?
            .to_string();

        let mut config_map: HashMap<String, String> = std::env::vars()
            .filter(|(k, _)| k.starts_with("BOS_") || k.starts_with("AWS_"))
            .map(|(k, v)| {
                let key = k.to_lowercase();
                if key.starts_with("bos_") {
                    let suffix = key.strip_prefix("bos_").unwrap();
                    let s3_key = match suffix {
                        "access_key_id" => "access_key_id",
                        "secret_access_key" => "secret_access_key",
                        "endpoint" => "endpoint",
                        "region" => "region",
                        "session_token" => "security_token",
                        _ => suffix,
                    };
                    (s3_key.to_string(), v)
                } else {
                    (key.replace("aws_", ""), v)
                }
            })
            .collect();

        config_map.insert("bucket".to_string(), bucket);

        if let Some(endpoint) = storage_options.0.get("fs.bos.endpoint") {
            config_map.insert(
                "endpoint".to_string(),
                convert_to_s3_endpoint(endpoint).ok_or_else(|| {
                    Error::invalid_input(format!("Invalid BOS endpoint: {}", endpoint))
                })?,
            );
            config_map.insert(
                "region".to_string(),
                extract_region(endpoint).ok_or_else(|| {
                    Error::invalid_input(format!(
                        "Cannot extract region from BOS endpoint: {}",
                        endpoint
                    ))
                })?,
            );
        }

        if let Some(access_key_id) = storage_options.0.get("fs.bos.access.key") {
            config_map.insert("access_key_id".to_string(), access_key_id.clone());
        }

        if let Some(secret_access_key) = storage_options.0.get("fs.bos.secret.access.key") {
            config_map.insert("secret_access_key".to_string(), secret_access_key.clone());
        }

        if let Some(token) = storage_options.0.get("fs.bos.session.token.key") {
            config_map.insert("session_token".to_string(), token.clone());
        }

        if !config_map.contains_key("endpoint") {
            return Err(Error::invalid_input(
                "BOS endpoint is required. Please provide 'fs.bos.endpoint' in storage options or set BOS_ENDPOINT environment variable",
            ));
        }

        let max_retries = storage_options.client_max_retries();
        let operator = Operator::from_iter::<S3>(config_map)
            .map_err(|e| {
                Error::invalid_input(format!("Failed to create BOS(S3) operator: {:?}", e))
            })?
            .layer(RetryLayer::new().with_max_times(max_retries).with_jitter())
            .finish();

        let opendal_store = Arc::new(OpendalStore::new(operator));

        let mut url = base_path;
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }

        Ok(ObjectStore {
            scheme: "bos".to_string(),
            inner: opendal_store,
            block_size,
            max_iop_size: *DEFAULT_MAX_IOP_SIZE,
            use_constant_size_upload_parts: params.use_constant_size_upload_parts,
            list_is_lexically_ordered: params.list_is_lexically_ordered.unwrap_or(true),
            io_parallelism: DEFAULT_CLOUD_IO_PARALLELISM,
            download_retry_count: storage_options.download_retry_count(),
            io_tracker: Default::default(),
            store_prefix: self.calculate_object_store_prefix(&url, params.storage_options())?,
        })
    }
}

/// Extract region from a BOS endpoint URL.
///
/// For example, `https://bj.bcebos.com` returns `"bj"`.
fn extract_region(endpoint: &str) -> Option<String> {
    let endpoint = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))?;
    let region = endpoint.strip_suffix(".bcebos.com")?;
    if region.is_empty() || region.contains('/') {
        return None;
    }
    Some(region.to_string())
}

/// Convert a BOS endpoint to an S3-compatible endpoint.
///
/// For example, `https://bj.bcebos.com` becomes `https://s3.bj.bcebos.com`.
fn convert_to_s3_endpoint(endpoint: &str) -> Option<String> {
    if let Some(rest) = endpoint.strip_prefix("https://") {
        Some(format!("https://s3.{}", rest))
    } else {
        endpoint
            .strip_prefix("http://")
            .map(|rest| format!("http://s3.{}", rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_region() {
        assert_eq!(extract_region("https://bj.bcebos.com"), Some("bj".into()));
        assert_eq!(extract_region("http://gz.bcebos.com"), Some("gz".into()));
        assert_eq!(extract_region("https://su.bcebos.com"), Some("su".into()));
        assert_eq!(extract_region("ftp://bj.bcebos.com"), None);
        assert_eq!(extract_region("not-a-url"), None);
    }

    #[test]
    fn test_convert_to_s3_endpoint() {
        assert_eq!(
            convert_to_s3_endpoint("https://bj.bcebos.com"),
            Some("https://s3.bj.bcebos.com".into())
        );
        assert_eq!(
            convert_to_s3_endpoint("http://gz.bcebos.com"),
            Some("http://s3.gz.bcebos.com".into())
        );
        assert_eq!(convert_to_s3_endpoint("ftp://bj.bcebos.com"), None);
    }

    #[test]
    fn test_bos_store_path() {
        let provider = BosStoreProvider;
        let url = Url::parse("bos://bucket/path/to/file").unwrap();
        let path = provider.extract_path(&url).unwrap();
        let expected_path = object_store::path::Path::from("path/to/file");
        assert_eq!(path, expected_path);
    }
}
