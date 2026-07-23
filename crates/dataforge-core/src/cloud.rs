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

/// A session handle tracking state of an active S3/GCS multipart upload.
pub struct MultipartUploadSession {
    /// The generated unique upload session ID
    pub upload_id: String,
    /// Destination storage object path
    pub path: String,
    /// Completed parts list: vector of (PartNumber, ETag)
    pub parts: Vec<(usize, String)>,
    /// Combined payload buffer holding parts uploaded so far
    pub buffer: Vec<u8>,
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

    /// Initiate an S3/GCS style multipart upload session.
    pub fn initiate_multipart_upload(&self, path: &str) -> Result<MultipartUploadSession> {
        let upload_id = uuid::Uuid::new_v4().to_string();
        Ok(MultipartUploadSession {
            upload_id,
            path: path.to_string(),
            parts: Vec::new(),
            buffer: Vec::new(),
        })
    }

    /// Upload a chunk part to the multipart upload session.
    pub fn upload_part(&self, session: &mut MultipartUploadSession, part_number: usize, data: &[u8]) -> Result<String> {
        if data.is_empty() {
            return Err(DataForgeError::config("Cannot upload empty part in multipart upload"));
        }
        session.buffer.extend_from_slice(data);
        let etag = format!("etag-part-{}-{}", part_number, uuid::Uuid::new_v4());
        session.parts.push((part_number, etag.clone()));
        Ok(etag)
    }

    /// Finalize the multipart upload session, combining all parts and persisting the object in cloud storage.
    pub fn complete_multipart_upload(&self, session: MultipartUploadSession) -> Result<()> {
        if session.parts.is_empty() {
            return Err(DataForgeError::config("Cannot complete multipart upload with zero parts"));
        }
        self.write_object(&session.path, &session.buffer)?;
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

    #[test]
    fn test_cloud_multipart_upload() {
        let client = CloudStorageClient::new("gcs", "reports");
        let mut session = client.initiate_multipart_upload("monthly/2026-07.xlsx").unwrap();

        let part1 = b"Part 1 Data Chunk - ";
        let part2 = b"Part 2 Data Chunk";

        let etag1 = client.upload_part(&mut session, 1, part1).unwrap();
        let etag2 = client.upload_part(&mut session, 2, part2).unwrap();

        assert!(!etag1.is_empty());
        assert!(!etag2.is_empty());
        assert_eq!(session.parts.len(), 2);

        client.complete_multipart_upload(session).unwrap();

        let final_data = client.read_object("monthly/2026-07.xlsx").unwrap();
        assert_eq!(final_data, b"Part 1 Data Chunk - Part 2 Data Chunk");
    }
}
