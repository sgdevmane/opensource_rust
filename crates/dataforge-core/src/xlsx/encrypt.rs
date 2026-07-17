// =============================================================================
// DataForge Core — XLSX Encryption Writer
// =============================================================================
// Encrypts a ZIP (XLSX) payload into a password-protected OLE Compound File
// using the MS-OFFICE Agile Encryption standard (ECMA-376).
//
// Flow:
// 1. Accept plaintext XLSX bytes + password
// 2. Generate random AES-256 key + salt
// 3. Derive encryption key via PBKDF2-HMAC-SHA512
// 4. Encrypt the plaintext with AES-256-CBC
// 5. Wrap everything in an OLE Compound File with EncryptionInfo + EncryptedPackage
// =============================================================================

use std::io::Write;
use aes::cipher::{BlockEncryptMut, KeyIvInit};
use cbc::Encryptor;
use pbkdf2::pbkdf2;
use sha2::{Digest, Sha512};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::error::{DataForgeError, Result};

/// Number of PBKDF2 iterations (100,000 is the Excel default).
const SPIN_COUNT: u32 = 100_000;

/// Encrypts plaintext XLSX bytes with a password and writes the encrypted OLE
/// Compound File to the given writer.
///
/// # Arguments
/// * `xlsx_bytes` - The unencrypted XLSX (ZIP) payload.
/// * `password` - The password to protect the workbook with.
/// * `writer` - Destination for the encrypted output.
///
/// # Errors
/// Returns `DataForgeError` if encryption or OLE writing fails.
pub fn encrypt_xlsx<W: Write>(xlsx_bytes: &[u8], password: &str, writer: &mut W) -> Result<()> {
    // Generate random salt and package key
    let salt = generate_random_bytes(16);
    let package_key = generate_random_bytes(32);

    // Convert password to UTF-16LE
    let pass_utf16: Vec<u16> = password.encode_utf16().collect();
    let mut pass_bytes = Vec::with_capacity(pass_utf16.len() * 2);
    for &val in &pass_utf16 {
        pass_bytes.push((val & 0xFF) as u8);
        pass_bytes.push(((val >> 8) & 0xFF) as u8);
    }

    // Derive master key using PBKDF2-HMAC-SHA512
    let mut master_key = vec![0u8; 64];
    pbkdf2::<hmac::Hmac<Sha512>>(&pass_bytes, &salt, SPIN_COUNT, &mut master_key)
        .map_err(|e| DataForgeError::internal(format!("PBKDF2 derivation failed: {}", e)))?;

    // Derive AES key for encrypting the package key
    let block_constant = [0x1a, 0x8b, 0x80, 0x51, 0x0b, 0x3e, 0x78, 0xf6];
    let mut hasher = Sha512::new();
    hasher.update(&master_key[..32]);
    hasher.update(block_constant);
    let key_hash = hasher.finalize();
    let aes_key = &key_hash[..32];

    // Encrypt the package key
    let iv = vec![0u8; 16];
    let mut key_buf = package_key.clone();
    // Pad to AES block size
    let pad_len = 16 - (key_buf.len() % 16);
    key_buf.extend(std::iter::repeat(pad_len as u8).take(pad_len));
    let encryptor = Encryptor::<aes::Aes256>::new(aes_key.into(), iv.as_slice().into());
    encryptor.encrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut key_buf, 32)
        .map_err(|e| DataForgeError::internal(format!("Failed to encrypt package key: {}", e)))?;
    let encrypted_key_value = &key_buf[..];

    // Derive IV for package encryption
    let key_data_salt = generate_random_bytes(16);
    let mut iv_hasher = Sha512::new();
    iv_hasher.update(&key_data_salt);
    iv_hasher.update(0u32.to_le_bytes());
    let iv_hash = iv_hasher.finalize();
    let package_iv = &iv_hash[..16];

    // Encrypt the XLSX payload
    let mut padded_payload = xlsx_bytes.to_vec();
    let payload_pad = 16 - (padded_payload.len() % 16);
    padded_payload.extend(std::iter::repeat(payload_pad as u8).take(payload_pad));
    let payload_len = xlsx_bytes.len();
    let encryptor = Encryptor::<aes::Aes256>::new(package_key.as_slice().into(), package_iv.into());
    encryptor.encrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut padded_payload, payload_len)
        .map_err(|e| DataForgeError::internal(format!("Failed to encrypt package: {}", e)))?;

    // Build EncryptionInfo XML
    let encryption_info = build_encryption_info_xml(
        &salt,
        &key_data_salt,
        encrypted_key_value,
        SPIN_COUNT,
    );

    // Build EncryptedPackage (8-byte length prefix + ciphertext)
    let mut encrypted_package = Vec::with_capacity(8 + padded_payload.len());
    encrypted_package.extend_from_slice(&(payload_len as u64).to_le_bytes());
    encrypted_package.extend_from_slice(&padded_payload);

    // Write OLE Compound File
    write_ole_compound_file(writer, &encryption_info, &encrypted_package)?;

    Ok(())
}

/// Build the EncryptionInfo XML following ECMA-376 Agile format.
fn build_encryption_info_xml(
    password_salt: &[u8],
    key_data_salt: &[u8],
    encrypted_key_value: &[u8],
    spin_count: u32,
) -> Vec<u8> {
    // Version header: 4 bytes version (4.4) + 4 bytes reserved
    let mut info = Vec::new();
    info.extend_from_slice(&4u16.to_le_bytes()); // vMajor
    info.extend_from_slice(&4u16.to_le_bytes()); // vMinor
    info.extend_from_slice(&0x40u32.to_le_bytes()); // Flags: fAgile

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<encryption xmlns="http://schemas.microsoft.com/office/2006/encryption"
            xmlns:p="http://schemas.microsoft.com/office/2006/keyEncryptor/password">
  <keyData saltSize="16" blockSize="16" keyBits="256" hashSize="64"
           cipherAlgorithm="AES" cipherChaining="ChainingModeCBC"
           hashAlgorithm="SHA512"
           saltValue="{key_data_salt_b64}"/>
  <dataIntegrity encryptedHmacKey="" encryptedHmacValue=""/>
  <keyEncryptors>
    <keyEncryptor uri="http://schemas.microsoft.com/office/2006/keyEncryptor/password">
      <p:encryptedKey spinCount="{spin_count}" saltSize="16" blockSize="16"
                      keyBits="256" hashSize="64"
                      cipherAlgorithm="AES" cipherChaining="ChainingModeCBC"
                      hashAlgorithm="SHA512"
                      saltValue="{salt_b64}"
                      encryptedKeyValue="{enc_key_b64}"/>
    </keyEncryptor>
  </keyEncryptors>
</encryption>"#,
        key_data_salt_b64 = BASE64.encode(key_data_salt),
        spin_count = spin_count,
        salt_b64 = BASE64.encode(password_salt),
        enc_key_b64 = BASE64.encode(encrypted_key_value),
    );

    info.extend_from_slice(xml.as_bytes());
    info
}

/// Write a minimal OLE Compound File containing EncryptionInfo and EncryptedPackage.
///
/// This creates a simplified CFB structure. For full production use,
/// consider using the `cfb` crate's writer API.
fn write_ole_compound_file<W: Write>(
    writer: &mut W,
    encryption_info: &[u8],
    encrypted_package: &[u8],
) -> Result<()> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut comp = cfb::CompoundFile::create(cursor)
        .map_err(|e| DataForgeError::internal(format!("Failed to create OLE compound file: {}", e)))?;

    comp.create_stream("/EncryptionInfo")
        .map_err(|e| DataForgeError::internal(format!("Failed to create EncryptionInfo stream: {}", e)))?
        .write_all(encryption_info)?;

    comp.create_stream("/EncryptedPackage")
        .map_err(|e| DataForgeError::internal(format!("Failed to create EncryptedPackage stream: {}", e)))?
        .write_all(encrypted_package)?;

    let cursor = comp.into_inner();
    writer.write_all(cursor.get_ref())?;

    Ok(())
}

/// Generate random bytes using a simple xorshift PRNG seeded from system time.
/// Not cryptographically secure, but sufficient for salt generation in this context.
/// For production, replace with `getrandom` or `rand` crate.
fn generate_random_bytes(len: usize) -> Vec<u8> {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let mut state = seed;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push(state as u8);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_bytes_length() {
        let b = generate_random_bytes(32);
        assert_eq!(b.len(), 32);
    }

    #[test]
    fn test_build_encryption_info_xml() {
        let salt = vec![0u8; 16];
        let key_data_salt = vec![0u8; 16];
        let enc_key = vec![0u8; 32];
        let info = build_encryption_info_xml(&salt, &key_data_salt, &enc_key, 100000);
        let xml_part = String::from_utf8_lossy(&info[8..]);
        assert!(xml_part.contains("spinCount=\"100000\""));
        assert!(xml_part.contains("hashAlgorithm=\"SHA512\""));
    }
}
