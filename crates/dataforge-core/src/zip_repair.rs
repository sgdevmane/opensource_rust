// =============================================================================
// DataForge Core — Zip Structural Auto-Correction Utility
// =============================================================================
// Scans corrupted ZIP files (e.g. truncated central directory) for local headers
// and repairs/reconstructs a valid ZIP archive structure.
// =============================================================================

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use crate::error::{DataForgeError, Result};

/// Scans a corrupted ZIP file, extracts valid local files, and writes a repaired archive.
/// Returns the path to the repaired ZIP archive (or the original if no repair was needed).
pub fn auto_correct_zip(path: &Path) -> Result<PathBuf> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    // If it opens successfully as-is, return original path
    if zip::ZipArchive::new(std::io::Cursor::new(&data)).is_ok() {
        return Ok(path.to_path_buf());
    }

    // Attempt repair: scan for local file headers "PK\x03\x04"
    let temp_file = tempfile::NamedTempFile::new().map_err(|e| {
        DataForgeError::io(e, "Failed to create temp file for ZIP repair")
    })?;
    let (temp_file_handle, temp_path) = temp_file.into_parts();
    let mut new_zip = zip::ZipWriter::new(temp_file_handle);

    let mut offset = 0;
    let mut recovered = 0;

    while offset < data.len() - 30 {
        if &data[offset..offset+4] == b"PK\x03\x04" {
            let filename_len = u16::from_le_bytes([data[offset+26], data[offset+27]]) as usize;
            let extra_len = u16::from_le_bytes([data[offset+28], data[offset+29]]) as usize;
            let comp_size = u32::from_le_bytes([
                data[offset+18], data[offset+19], data[offset+20], data[offset+21]
            ]) as usize;

            if offset + 30 + filename_len + extra_len + comp_size <= data.len() {
                let filename_bytes = &data[offset+30..offset+30+filename_len];
                if let Ok(name) = std::str::from_utf8(filename_bytes) {
                    if !name.is_empty() && !name.ends_with('/') {
                        let payload_start = offset + 30 + filename_len + extra_len;
                        let payload = &data[payload_start..payload_start+comp_size];

                        let options = zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored);

                        if new_zip.start_file(name, options).is_ok() {
                            let _ = new_zip.write_all(payload);
                            recovered += 1;
                        }
                    }
                }
            }
        }
        offset += 1;
    }

    if recovered == 0 {
        return Err(DataForgeError::config("ZIP structural auto-correction failed: no local headers recovered"));
    }

    new_zip.finish().map_err(|e| {
        DataForgeError::internal(format!("Failed to finalize repaired ZIP archive: {e}"))
    })?;

    let path = temp_path.keep().map_err(|e| {
        DataForgeError::internal(format!("Failed to persist repaired ZIP archive: {e}"))
    })?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zip_auto_correction() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("corrupted.zip");

        // Write a valid zip first
        {
            let file = File::create(&file_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("test.txt", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"Hello Zip").unwrap();
            zip.finish().unwrap();
        }

        // Corrupt it by appending garbage bytes and truncating some central directory references
        {
            let mut data = std::fs::read(&file_path).unwrap();
            if data.len() > 10 {
                data.truncate(data.len() - 10); // Remove central directory end headers
            }
            std::fs::write(&file_path, &data).unwrap();
        }

        // Run auto-correction
        let repaired_path = auto_correct_zip(&file_path).unwrap();
        assert!(repaired_path.exists());

        // Validate we can read it now!
        let rep_file = File::open(repaired_path).unwrap();
        let mut archive = zip::ZipArchive::new(rep_file).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive.by_index(0).unwrap().name(), "test.txt");
    }
}
