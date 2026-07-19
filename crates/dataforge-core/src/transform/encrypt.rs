// =============================================================================
// DataForge Core — Column-wise Encrypted Export Transform
// =============================================================================
// Encrypts selected columns in a RowBatch using AES-256-CBC and Base64 encoding.
// =============================================================================

use crate::types::{CellValue, RowBatch};
use crate::error::{DataForgeError, Result};
use aes::cipher::{block_padding::Pkcs7, KeyIvInit, BlockEncryptMut};
use cbc::Encryptor;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

type Aes256CbcEnc = Encryptor<aes::Aes256>;

/// Encrypt data using AES-256-CBC with PKCS7 padding.
pub fn encrypt_aes_256_cbc(data: &[u8], key: &[u8; 32], iv: &[u8; 16]) -> Result<Vec<u8>> {
    let encryptor = Aes256CbcEnc::new(key.into(), iv.into());
    // Buffer size: data length + block size (16)
    let mut buf = vec![0u8; data.len() + 16];
    buf[..data.len()].copy_from_slice(data);
    let ciphertext = encryptor.encrypt_padded_mut::<Pkcs7>(&mut buf, data.len())
        .map_err(|e| DataForgeError::internal(format!("Encryption padding error: {e}")))?;
    Ok(ciphertext.to_vec())
}

/// Encrypt values in specified columns of a RowBatch using AES-256-CBC, outputting Base64 strings.
pub fn encrypt_columns(batch: &mut RowBatch, columns: &[usize], key: &[u8; 32], iv: &[u8; 16]) -> Result<()> {
    for row in &mut batch.rows {
        for &col_idx in columns {
            if let Some(cell) = row.get_mut(col_idx) {
                if !cell.is_null() {
                    let plaintext = cell.to_display_string();
                    let enc_data = encrypt_aes_256_cbc(plaintext.as_bytes(), key, iv)?;
                    let encoded = BASE64.encode(enc_data);
                    *cell = CellValue::from(encoded);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Row;

    #[test]
    fn test_column_encryption() {
        let key = [0x42u8; 32];
        let iv = [0x24u8; 16];

        let mut batch = RowBatch::new(0);
        batch.headers = Some(vec!["name".to_string(), "ssn".to_string()]);

        let mut row = Row::new(0);
        row.push(CellValue::from("Alice"));
        row.push(CellValue::from("123-456-7890"));
        batch.push(row);

        encrypt_columns(&mut batch, &[1], &key, &iv).unwrap();

        let ssn_val = batch.rows[0].get_str(1).unwrap();
        assert_ne!(ssn_val, "123-456-7890"); // SSN is encrypted!
        assert!(!ssn_val.is_empty());
    }
}
