// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Lance Authors

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use object_store_opendal::OpendalStore;
use opendal::{Operator, services::S3};
use regex::Regex;
use url::Url;

use crate::object_store::{
    DEFAULT_CLOUD_BLOCK_SIZE, DEFAULT_CLOUD_IO_PARALLELISM, DEFAULT_MAX_IOP_SIZE, ObjectStore,
    ObjectStoreParams, ObjectStoreProvider, StorageOptions,
};
use lance_core::error::{Error, Result};

/// BOS endpoints are `<region>.bcebos.com`, so the region is the first label of the host.
static RE_REGION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://([^.]+)\.bcebos\.com").unwrap());

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
            .filter_map(|(key, value)| normalize_s3_env_var(key.as_str()).map(|key| (key, value)))
            .collect();

        config_map.insert("bucket".to_string(), bucket);

        if let Some(endpoint) = storage_options.0.get("fs.bos.endpoint") {
            let s3_endpoint = convert_to_s3_endpoint(endpoint).ok_or_else(|| {
                Error::invalid_input(format!(
                    "Invalid BOS endpoint format: '{endpoint}', expected 'http(s)://...'"
                ))
            })?;
            let region = extract_region(endpoint).ok_or_else(|| {
                Error::invalid_input(format!(
                    "Cannot extract region from BOS endpoint: '{endpoint}', expected 'http(s)://<region>.bcebos.com'"
                ))
            })?;
            config_map.insert("endpoint".to_string(), s3_endpoint);
            config_map.insert("region".to_string(), region);
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

        let operator = Operator::from_iter::<S3>(config_map)
            .map_err(|e| {
                Error::invalid_input(format!("Failed to create BOS(S3) operator: {:?}", e))
            })?
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

fn extract_region(endpoint: &str) -> Option<String> {
    RE_REGION
        .captures(endpoint)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

fn normalize_s3_env_var(key: &str) -> Option<String> {
    let key = key.to_lowercase();
    if let Some(suffix) = key.strip_prefix("bos_") {
        // The recognized BOS_* env vars already share their names with the
        // OpenDAL S3 config keys.
        Some(suffix.to_string())
    } else {
        key.strip_prefix("aws_").map(|suffix| suffix.to_string())
    }
}

fn convert_to_s3_endpoint(endpoint: &str) -> Option<String> {
    endpoint
        .strip_prefix("https://")
        .map(|rest| format!("https://s3.{}", rest))
        .or_else(|| {
            endpoint
                .strip_prefix("http://")
                .map(|rest| format!("http://s3.{}", rest))
        })
}

#[cfg(test)]
mod tests {
    use super::{convert_to_s3_endpoint, extract_region, normalize_s3_env_var};

    #[test]
    fn test_convert_to_s3_endpoint() {
        assert_eq!(
            Some("https://s3.cn-north-1.bcebos.com".to_string()),
            convert_to_s3_endpoint("https://cn-north-1.bcebos.com")
        );
        assert_eq!(
            Some("http://s3.cn-north-1.bcebos.com".to_string()),
            convert_to_s3_endpoint("http://cn-north-1.bcebos.com")
        );
        assert_eq!(None, convert_to_s3_endpoint("ftp://cn-north-1.bcebos.com"));
    }

    #[test]
    fn test_extract_region() {
        assert_eq!(
            Some("cn-north-1".to_string()),
            extract_region("https://cn-north-1.bcebos.com")
        );
        assert_eq!(None, extract_region("https://example.com"));
    }

    #[test]
    fn test_normalize_s3_env_var() {
        assert_eq!(
            Some("access_key_id".to_string()),
            normalize_s3_env_var("BOS_ACCESS_KEY_ID")
        );
        assert_eq!(
            Some("region".to_string()),
            normalize_s3_env_var("AWS_REGION")
        );
        assert_eq!(None, normalize_s3_env_var("OTHER_KEY"));
    }

    /// Commits on BOS rely on conditional puts (`If-None-Match: *`) to detect
    /// concurrent writers. BOS documents the header on its native PutObject; this
    /// checks the S3-compatible endpoint we actually talk to, so it needs a real
    /// bucket and only runs when asked for:
    ///
    /// ```bash
    /// BOS_ACCESS_KEY_ID=... BOS_SECRET_ACCESS_KEY=... \
    ///     BOS_TEST_BUCKET=my-bucket BOS_TEST_ENDPOINT=https://bj.bcebos.com \
    ///     cargo test -p lance-io --features bos conditional_put -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "requires a real BOS bucket"]
    async fn test_conditional_put_is_honored() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use object_store::path::Path;
        use object_store::{
            ObjectStore as _, ObjectStoreExt as _, PutMode, PutOptions, PutPayload,
        };
        use url::Url;

        use super::BosStoreProvider;
        use crate::object_store::{ObjectStoreParams, ObjectStoreProvider, StorageOptionsAccessor};

        let bucket = std::env::var("BOS_TEST_BUCKET").expect("BOS_TEST_BUCKET is required");
        let endpoint = std::env::var("BOS_TEST_ENDPOINT").expect("BOS_TEST_ENDPOINT is required");

        let params = ObjectStoreParams {
            storage_options_accessor: Some(Arc::new(StorageOptionsAccessor::with_static_options(
                HashMap::from([("fs.bos.endpoint".to_string(), endpoint)]),
            ))),
            ..Default::default()
        };
        let store = BosStoreProvider
            .new_store(Url::parse(&format!("bos://{bucket}/")).unwrap(), &params)
            .await
            .unwrap();

        let path = Path::from(format!(
            "lance-conditional-put-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let create = || {
            store.inner.put_opts(
                &path,
                PutPayload::from_static(b"manifest"),
                PutOptions {
                    mode: PutMode::Create,
                    ..Default::default()
                },
            )
        };

        create()
            .await
            .expect("first conditional put should succeed");
        let err = create().await.expect_err(
            "BOS accepted an overwrite under `If-None-Match: *`, so \
             ConditionalPutCommitHandler cannot detect commit conflicts",
        );
        assert!(
            matches!(
                err,
                object_store::Error::AlreadyExists { .. }
                    | object_store::Error::Precondition { .. }
            ),
            "conditional put failed for an unexpected reason: {err}"
        );

        store.inner.delete(&path).await.unwrap();
    }
}
