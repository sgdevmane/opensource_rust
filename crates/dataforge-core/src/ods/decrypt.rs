// =============================================================================
// DataForge Core — ODS Encryption Support (ODF Manifest Decryption)
// =============================================================================
// Decrypts password-protected OpenDocument Spreadsheet (ODS) entries by
// parsing META-INF/manifest.xml and applying PBKDF2 key derivation and
// AES-256-CBC decryption.
// =============================================================================

use aes::cipher::{BlockDecryptMut, KeyIvInit};
use cbc::Decryptor;
use pbkdf2::pbkdf2;
use sha2::Sha256;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use quick_xml::events::Event;
use quick_xml::reader::Reader as XmlReader;

use crate::error::{DataForgeError, Result};

/// Holds encryption metadata parsed from ODF manifest.xml for a specific file.
#[derive(Debug, Clone, Default)]
pub struct EncryptionData {
    pub algorithm: String,
    pub iv: Vec<u8>,
    pub salt: Vec<u8>,
    pub iteration_count: u32,
    pub key_size: u32,
}

/// Parses the ODF manifest.xml file to find encryption details for a target file.
pub fn parse_manifest_encryption(manifest_xml: &[u8], target_path: &str) -> Result<Option<EncryptionData>> {
    let mut reader = XmlReader::from_reader(manifest_xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut current_path = String::new();
    let mut current_enc: Option<EncryptionData> = None;
    let mut inside_enc = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                match e.name().as_ref() {
                    b"manifest:file-entry" => {
                        current_path = String::new();
                        current_enc = None;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"manifest:full-path" {
                                current_path = attr.decode_and_unescape_value(reader.decoder())?.to_string();
                            }
                        }
                    }
                    b"manifest:encryption-data" => {
                        inside_enc = true;
                        current_enc = Some(EncryptionData::default());
                    }
                    b"manifest:algorithm" if inside_enc => {
                        if let Some(ref mut enc) = current_enc {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"manifest:algorithm-name" {
                                    enc.algorithm = attr.decode_and_unescape_value(reader.decoder())?.to_string();
                                } else if attr.key.as_ref() == b"manifest:initialisation-vector" {
                                    let iv_b64 = attr.decode_and_unescape_value(reader.decoder())?;
                                    enc.iv = BASE64.decode(iv_b64.as_ref()).unwrap_or_default();
                                }
                            }
                        }
                    }
                    b"manifest:key-derivation" if inside_enc => {
                        if let Some(ref mut enc) = current_enc {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"manifest:salt" {
                                    let salt_b64 = attr.decode_and_unescape_value(reader.decoder())?;
                                    enc.salt = BASE64.decode(salt_b64.as_ref()).unwrap_or_default();
                                } else if attr.key.as_ref() == b"manifest:iteration-count" {
                                    let count_str = attr.decode_and_unescape_value(reader.decoder())?;
                                    enc.iteration_count = count_str.parse().unwrap_or(1024);
                                } else if attr.key.as_ref() == b"manifest:key-size" {
                                    let size_str = attr.decode_and_unescape_value(reader.decoder())?;
                                    enc.key_size = size_str.parse().unwrap_or(32);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                match e.name().as_ref() {
                    b"manifest:file-entry" => {
                        if current_path == target_path && current_enc.is_some() {
                            return Ok(current_enc);
                        }
                    }
                    b"manifest:encryption-data" => {
                        inside_enc = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(DataForgeError::OdsParse {
                component: "manifest".to_string(),
                message: format!("Manifest XML error: {e}"),
            }),
            _ => {}
        }
        buf.clear();
    }

    Ok(None)
}

/// Decrypts the raw encrypted bytes using the derived key and initialization vector.
pub fn decrypt_ods_entry(
    encrypted_bytes: &[u8],
    password: &str,
    enc_data: &EncryptionData,
) -> Result<Vec<u8>> {
    // Derive password key using PBKDF2-HMAC-SHA256
    let mut derived_key = vec![0u8; enc_data.key_size as usize];
    pbkdf2::<hmac::Hmac<Sha256>>(
        password.as_bytes(),
        &enc_data.salt,
        enc_data.iteration_count,
        &mut derived_key,
    ).map_err(|e| DataForgeError::internal(format!("ODS PBKDF2 derivation failed: {e}")))?;

    // Decrypt payload using AES-256-CBC (or matching algorithm)
    let mut decrypted_payload = encrypted_bytes.to_vec();
    
    // ODF files might use padding. AES block size is 16.
    let decryptor = Decryptor::<aes::Aes256>::new(derived_key.as_slice().into(), enc_data.iv.as_slice().into());
    let decrypted_len = decryptor.decrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut decrypted_payload)
        .map_err(|e| DataForgeError::internal(format!("ODS AES decryption failed: {e}")))?
        .len();

    decrypted_payload.truncate(decrypted_len);
    Ok(decrypted_payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest_xml() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0">
  <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml">
    <manifest:encryption-data manifest:checksum-type="SHA256" manifest:checksum="1234">
      <manifest:algorithm manifest:algorithm-name="http://www.w3.org/2001/04/xmlenc#aes256-cbc" manifest:initialisation-vector="MTIzNDU2Nzg5MDEyMzQ1Ng=="/>
      <manifest:key-derivation manifest:key-derivation-name="PBKDF2" manifest:key-size="32" manifest:iteration-count="1024" manifest:salt="c2FsdF9zYWx0X3NhbHRfc2FsdA=="/>
    </manifest:encryption-data>
  </manifest:file-entry>
</manifest:manifest>"#;

        let res = parse_manifest_encryption(xml, "content.xml").unwrap();
        assert!(res.is_some());
        let enc = res.unwrap();
        assert_eq!(enc.iteration_count, 1024);
        assert_eq!(enc.key_size, 32);
        assert_eq!(enc.salt, b"salt_salt_salt_salt");
        assert_eq!(enc.iv, b"1234567890123456");
    }
}
