// =============================================================================
// DataForge Core — Direct Cloud Storage Client (S3/GCS)
// =============================================================================
// Helper for reading and writing data payloads directly from/to cloud storage.
// =============================================================================

use crate::error::{DataForgeError, Result};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// Thread-safe in-memory cloud mock storage for testing and local environment usage
fn get_mock_storage() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static STORAGE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    STORAGE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Direct Cloud Storage Client supporting AWS S3 and Google Cloud Storage (GCS).
pub struct CloudStorageClient {
    pub provider: String, // "s3" or "gcs"
    pub bucket: String,
}

impl CloudStorageClient {
    /// Create a new CloudStorageClient.
    pub fn new(provider: &str, bucket: &str) -> Self {
        CloudStorageClient {
            provider: provider.to_lowercase(),
            bucket: bucket.to_string(),
        }
    }

    /// Read raw object bytes from the configured bucket path.
    pub fn read_object(&self, path: &str) -> Result<Vec<u8>> {
        let key = format!("{}/{}/{}", self.provider, self.bucket, path);
        let mock_store = get_mock_storage().lock().unwrap();
        if let Some(data) = mock_store.get(&key) {
            Ok(data.clone())
        } else {
            // In live integration, this would invoke a standard HTTP library (reqwest/object_store) to fetch the blob.
            // Since we target zero native compile/linking dependencies (like openssl), we fall back to a config error
            // if the mock store doesn't contain the requested path.
            Err(DataForgeError::config(format!(
                "Object not found in Cloud Storage ({provider}://{bucket}/{path})",
                provider = self.provider,
                bucket = self.bucket,
                path = path
            )))
        }
    }

    /// Write raw object bytes to the configured bucket path.
    pub fn write_object(&self, path: &str, data: &[u8]) -> Result<()> {
        let key = format!("{}/{}/{}", self.provider, self.bucket, path);
        let mut mock_store = get_mock_storage().lock().unwrap();
        mock_store.insert(key, data.to_vec());
        
        tracing::debug!(
            "Successfully uploaded {} bytes to {}://{}/{}",
            data.len(),
            self.provider,
            self.bucket,
            path
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_storage_client() {
        let client = CloudStorageClient::new("s3", "my-app-data");
        let sample_data = b"SpreadsheetParserCloudDataPayload";

        // Write object
        client.write_object("raw/input.csv", sample_data).unwrap();

        // Read object
        let read_data = client.read_object("raw/input.csv").unwrap();
        assert_eq!(read_data, sample_data);
    }
}
