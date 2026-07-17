// =============================================================================
// DataForge Core — XLSX Password Decrypter
// =============================================================================
// Standard MS-OFFICE Agile Decryption (ECMA-376) for password-protected workbooks.
// =============================================================================

use std::io::{Cursor, Read};
use cfb::CompoundFile;
use aes::cipher::{BlockDecryptMut, KeyIvInit};
use cbc::Decryptor;
use pbkdf2::pbkdf2;
use sha2::{Digest, Sha512};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::error::{DataForgeError, Result};

/// Decrypter for password-protected XLSX workbooks.
pub struct XlsxDecrypter;

impl XlsxDecrypter {
    /// Check if the bytes start with the CFB Compound File Binary magic bytes.
    pub fn is_encrypted(data: &[u8]) -> bool {
        data.len() >= 8 && data[..8] == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
    }

    /// Attempt to decrypt a password-protected XLSX file.
    /// Returns the decrypted ZIP bytes if successful.
    pub fn decrypt(data: &[u8], password: &str) -> Result<Vec<u8>> {
        if !Self::is_encrypted(data) {
            return Err(DataForgeError::xlsx_parse("xlsx", "File is not a password-protected OLE document"));
        }

        let mut comp = CompoundFile::open(Cursor::new(data))
            .map_err(|e| DataForgeError::xlsx_parse("cfb", format!("Failed to parse OLE compound file: {}", e)))?;

        // Read EncryptionInfo XML
        let mut info_bytes = Vec::new();
        comp.open_stream("/EncryptionInfo")
            .map_err(|e| DataForgeError::xlsx_parse("cfb", format!("Missing EncryptionInfo stream: {}", e)))?
            .read_to_end(&mut info_bytes)?;

        // Read EncryptedPackage ciphertext
        let mut package_bytes = Vec::new();
        comp.open_stream("/EncryptedPackage")
            .map_err(|e| DataForgeError::xlsx_parse("cfb", format!("Missing EncryptedPackage stream: {}", e)))?
            .read_to_end(&mut package_bytes)?;

        if package_bytes.len() < 8 {
            return Err(DataForgeError::xlsx_parse("package", "Encrypted package stream is too short"));
        }

        let unencrypted_len = u64::from_le_bytes(package_bytes[..8].try_into().unwrap());
        let ciphertext = &package_bytes[8..];

        // Parse key derive elements from EncryptionInfo XML
        let info_str = String::from_utf8_lossy(&info_bytes);
        let spin_count = extract_attr(&info_str, "spinCount").and_then(|s| s.parse::<u32>().ok()).unwrap_or(100000);
        let salt_val = extract_attr(&info_str, "saltValue").ok_or_else(|| DataForgeError::config("Missing saltValue"))?;
        let enc_key_val = extract_attr(&info_str, "encryptedKeyValue").ok_or_else(|| DataForgeError::config("Missing encryptedKeyValue"))?;

        let salt = BASE64.decode(salt_val.trim())
            .map_err(|_| DataForgeError::config("Invalid base64 salt"))?;
        let encrypted_key = BASE64.decode(enc_key_val.trim())
            .map_err(|_| DataForgeError::config("Invalid base64 encrypted key"))?;

        // Convert password to UTF-16LE bytes
        let pass_utf16: Vec<u16> = password.encode_utf16().collect();
        let mut pass_bytes = Vec::with_capacity(pass_utf16.len() * 2);
        for &val in &pass_utf16 {
            pass_bytes.push((val & 0xFF) as u8);
            pass_bytes.push(((val >> 8) & 0xFF) as u8);
        }

        // PBKDF2/SHA-512 derivation of master key
        let mut master_key = vec![0u8; 64];
        pbkdf2::<hmac::Hmac<Sha512>>(&pass_bytes, &salt, spin_count, &mut master_key)
            .map_err(|e| DataForgeError::internal(format!("PBKDF2 derivation failed: {}", e)))?;

        // Derive block key for password decryption
        let block_constant = [0x1a, 0x8b, 0x80, 0x51, 0x0b, 0x3e, 0x78, 0xf6];
        let mut hasher = Sha512::new();
        hasher.update(&master_key[..32]);
        hasher.update(block_constant);
        let key_hash = hasher.finalize();

        // Decrypt package key
        let aes_key = &key_hash[..32];
        let mut decrypt_buf = encrypted_key.clone();
        let iv = vec![0u8; 16];
        let decryptor = Decryptor::<aes::Aes256>::new(aes_key.into(), iv.as_slice().into());
        let decrypted_key = decryptor.decrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut decrypt_buf)
            .map_err(|e| DataForgeError::xlsx_parse("decrypt", format!("Failed to decrypt package key: {}", e)))?;

        // Decrypt ciphertext using package key and derived IV
        let key_data_salt_val = extract_key_data_salt(&info_str).ok_or_else(|| DataForgeError::config("Missing keyData saltValue"))?;
        let key_data_salt = BASE64.decode(key_data_salt_val.trim())
            .map_err(|_| DataForgeError::config("Invalid base64 keyData salt"))?;

        let mut iv_hasher = Sha512::new();
        iv_hasher.update(&key_data_salt);
        iv_hasher.update(0u32.to_le_bytes());
        let iv_hash = iv_hasher.finalize();
        let package_iv = &iv_hash[..16];

        let mut package_decrypt_buf = ciphertext.to_vec();
        let package_decryptor = Decryptor::<aes::Aes256>::new(decrypted_key.into(), package_iv.into());
        let decrypted_package = package_decryptor.decrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut package_decrypt_buf)
            .map_err(|e| DataForgeError::xlsx_parse("decrypt", format!("Failed to decrypt package: {}", e)))?;

        if decrypted_package.len() > unencrypted_len as usize {
            package_decrypt_buf.truncate(unencrypted_len as usize);
            return Ok(package_decrypt_buf);
        }

        Ok(decrypted_package.to_vec())
    }
}

fn extract_attr(xml: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    if let Some(pos) = xml.find(&pattern) {
        let start = pos + pattern.len();
        if let Some(end) = xml[start..].find('"') {
            return Some(xml[start..start + end].to_string());
        }
    }
    None
}

fn extract_key_data_salt(xml: &str) -> Option<String> {
    if let Some(pos) = xml.find("<keyData") {
        let tag_content = &xml[pos..];
        if let Some(end_pos) = tag_content.find('>') {
            return extract_attr(&tag_content[..end_pos], "saltValue");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_encrypted() {
        // Plain text
        assert!(!XlsxDecrypter::is_encrypted(b"plain_text"));

        // CFB Magic
        let cfb_magic = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        assert!(XlsxDecrypter::is_encrypted(&cfb_magic));
    }
}
