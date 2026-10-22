//! Encrypting parts of a request body before it is sent, and decrypting parts of the response, for APIs that exchange
//! encrypted fields (card numbers, personal data) or whole encrypted payloads.
//!
//! AES in GCM (authenticated, recommended) or CBC (PKCS#7 padding) mode, with a 128- or 256-bit key from a variable,
//! so the key can be a secret. Each value gets a fresh random IV, written in front of the ciphertext (`iv ‖ ciphertext
//! [‖ tag]`) and encoded as base64 or hex, unless a fixed IV is given. Fields are chosen by JSON path (`$.card.number`,
//! `$.items[*].ssn`); `$` is the whole body. A non-string value is encrypted as its JSON text. A decrypted value comes
//! back as a string, or as JSON when it is a JSON object or array.

use aes_gcm::aead::{Aead, KeyInit};
use base64::Engine;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum Algorithm {
    #[default]
    #[serde(rename = "aes-256-gcm")]
    Aes256Gcm,
    #[serde(rename = "aes-128-gcm")]
    Aes128Gcm,
    #[serde(rename = "aes-256-cbc")]
    Aes256Cbc,
    #[serde(rename = "aes-128-cbc")]
    Aes128Cbc,
}

impl Algorithm {
    fn key_len(self) -> usize {
        match self {
            Algorithm::Aes256Gcm | Algorithm::Aes256Cbc => 32,
            Algorithm::Aes128Gcm | Algorithm::Aes128Cbc => 16,
        }
    }

    fn iv_len(self) -> usize {
        if self.gcm() { 12 } else { 16 }
    }

    fn gcm(self) -> bool {
        matches!(self, Algorithm::Aes256Gcm | Algorithm::Aes128Gcm)
    }
}

/// How bytes are written as text: the key and IV as given, and the ciphertext.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Base64,
    Hex,
    /// The text's own bytes (keys and IVs only).
    Utf8,
}

/// A request's encryption settings, as written in its file. `key` and `iv` may use `{{variables}}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CryptoSettings {
    pub algorithm: Algorithm,
    /// The key, such as `{{encryptionKey}}` (a secret variable).
    pub key: String,
    #[serde(skip_serializing_if = "is_base64")]
    pub key_encoding: Encoding,
    /// A fixed IV; empty means a random one for every value, written before the ciphertext.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub iv: String,
    /// How the ciphertext is written: base64 or hex.
    #[serde(skip_serializing_if = "is_base64")]
    pub output: Encoding,
    /// JSON paths of the body fields to encrypt before sending; `$` is the whole body.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub encrypt: Vec<String>,
    /// JSON paths of the response fields to decrypt; `$` is the whole body.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decrypt: Vec<String>,
}

fn is_base64(e: &Encoding) -> bool {
    *e == Encoding::Base64
}

impl CryptoSettings {
    pub fn is_empty(&self) -> bool {
        self.encrypt.is_empty() && self.decrypt.is_empty()
    }
}

fn decode(text: &str, encoding: Encoding, what: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    match encoding {
        Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
        Encoding::Hex => {
            if !text.len().is_multiple_of(2) {
                return Err(format!(
                    "the {what} is not hex: it has an odd number of digits"
                ));
            }
            (0..text.len())
                .step_by(2)
                .map(|i| {
                    u8::from_str_radix(&text[i..i + 2], 16)
                        .map_err(|_| format!("the {what} is not hex"))
                })
                .collect()
        }
        Encoding::Base64 => {
            let engines = [
                &base64::engine::general_purpose::STANDARD,
                &base64::engine::general_purpose::URL_SAFE,
                &base64::engine::general_purpose::STANDARD_NO_PAD,
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            ];
            engines
                .iter()
                .find_map(|e| e.decode(text).ok())
                .ok_or_else(|| format!("the {what} is not base64"))
        }
    }
}

fn encode(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Hex => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        _ => base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

/// Settings with the key and IV as bytes, checked once per request.
pub struct Cipher {
    algorithm: Algorithm,
    key: Vec<u8>,
    iv: Option<Vec<u8>>,
    output: Encoding,
}

// Without the key, so it never ends up in a log.
impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cipher({})", name(self.algorithm))
    }
}

impl Cipher {
    /// `key` and `iv` are the settings' values after variables.
    pub fn new(settings: &CryptoSettings, key: &str, iv: &str) -> Result<Self, String> {
        if key.trim().is_empty() {
            return Err("no encryption key: set one (such as {{encryptionKey}}) in the request's Crypto tab".into());
        }
        let key = decode(key, settings.key_encoding, "key")?;
        let algorithm = settings.algorithm;
        if key.len() != algorithm.key_len() {
            return Err(format!(
                "the key is {} bytes, but {} needs {} ({} bits)",
                key.len(),
                name(algorithm),
                algorithm.key_len(),
                algorithm.key_len() * 8
            ));
        }
        let iv = if iv.trim().is_empty() {
            None
        } else {
            let iv = decode(iv, settings.key_encoding, "IV")?;
            if iv.len() != algorithm.iv_len() {
                return Err(format!(
                    "the IV is {} bytes; {} needs {}",
                    iv.len(),
                    name(algorithm),
                    algorithm.iv_len()
                ));
            }
            Some(iv)
        };
        let output = if settings.output == Encoding::Hex {
            Encoding::Hex
        } else {
            Encoding::Base64
        };
        Ok(Self {
            algorithm,
            key,
            iv,
            output,
        })
    }

    pub fn encrypt(&self, plain: &[u8]) -> Result<String, String> {
        let iv = match &self.iv {
            Some(iv) => iv.clone(),
            None => {
                let mut iv = vec![0u8; self.algorithm.iv_len()];
                SystemRandom::new()
                    .fill(&mut iv)
                    .map_err(|_| "no random numbers available".to_string())?;
                iv
            }
        };
        let sealed = match self.algorithm {
            Algorithm::Aes256Gcm => aes_gcm::Aes256Gcm::new_from_slice(&self.key)
                .map_err(|e| e.to_string())?
                .encrypt(iv.as_slice().try_into().map_err(|_| "bad IV")?, plain),
            Algorithm::Aes128Gcm => aes_gcm::Aes128Gcm::new_from_slice(&self.key)
                .map_err(|e| e.to_string())?
                .encrypt(iv.as_slice().try_into().map_err(|_| "bad IV")?, plain),
            Algorithm::Aes256Cbc => Ok(cbc::Encryptor::<aes::Aes256>::new_from_slices(
                &self.key, &iv,
            )
            .map_err(|e| e.to_string())?
            .encrypt_padded_vec::<Pkcs7>(plain)),
            Algorithm::Aes128Cbc => Ok(cbc::Encryptor::<aes::Aes128>::new_from_slices(
                &self.key, &iv,
            )
            .map_err(|e| e.to_string())?
            .encrypt_padded_vec::<Pkcs7>(plain)),
        }
        .map_err(|_| "encryption failed".to_string())?;
        let out = if self.iv.is_some() {
            sealed
        } else {
            [iv, sealed].concat()
        };
        Ok(encode(&out, self.output))
    }

    pub fn decrypt(&self, text: &str) -> Result<Vec<u8>, String> {
        let bytes = decode(text, self.output, "value to decrypt")?;
        let (iv, data) = match &self.iv {
            Some(iv) => (iv.as_slice(), bytes.as_slice()),
            None if bytes.len() > self.algorithm.iv_len() => {
                bytes.split_at(self.algorithm.iv_len())
            }
            None => return Err("the value is too short to hold an IV and ciphertext".into()),
        };
        let wrong = || {
            format!(
                "could not decrypt: wrong key, IV, algorithm ({}) or encoding, or the value is not encrypted",
                name(self.algorithm)
            )
        };
        match self.algorithm {
            Algorithm::Aes256Gcm => aes_gcm::Aes256Gcm::new_from_slice(&self.key)
                .map_err(|e| e.to_string())?
                .decrypt(iv.try_into().map_err(|_| wrong())?, data)
                .map_err(|_| wrong()),
            Algorithm::Aes128Gcm => aes_gcm::Aes128Gcm::new_from_slice(&self.key)
                .map_err(|e| e.to_string())?
                .decrypt(iv.try_into().map_err(|_| wrong())?, data)
                .map_err(|_| wrong()),
            Algorithm::Aes256Cbc => cbc::Decryptor::<aes::Aes256>::new_from_slices(&self.key, iv)
                .map_err(|e| e.to_string())?
                .decrypt_padded_vec::<Pkcs7>(data)
                .map_err(|_| wrong()),
            Algorithm::Aes128Cbc => cbc::Decryptor::<aes::Aes128>::new_from_slices(&self.key, iv)
                .map_err(|e| e.to_string())?
                .decrypt_padded_vec::<Pkcs7>(data)
                .map_err(|_| wrong()),
        }
    }
}

fn name(a: Algorithm) -> &'static str {
    match a {
        Algorithm::Aes256Gcm => "AES-256-GCM",
        Algorithm::Aes128Gcm => "AES-128-GCM",
        Algorithm::Aes256Cbc => "AES-256-CBC",
        Algorithm::Aes128Cbc => "AES-128-CBC",
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Key(String),
    Index(usize),
    All,
}

/// `$.a.b[0].c`, `$.items[*].id`, `$['odd key']` → segments. `$` alone is the whole value.
fn parse_path(path: &str) -> Result<Vec<Segment>, String> {
    let p = path.trim();
    let rest = p
        .strip_prefix('$')
        .ok_or_else(|| format!("{p}: a JSON path starts with $, such as $.card.number"))?;
    let mut out = Vec::new();
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end] != '.' && chars[end] != '[' {
                    end += 1;
                }
                let key: String = chars[start..end].iter().collect();
                if key.is_empty() {
                    return Err(format!("{p}: empty name after a dot"));
                }
                out.push(if key == "*" {
                    Segment::All
                } else {
                    Segment::Key(key)
                });
                i = end;
            }
            '[' => {
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == ']')
                    .map(|e| i + e)
                    .ok_or_else(|| format!("{p}: missing ]"))?;
                let inner: String = chars[i + 1..end].iter().collect();
                let inner = inner.trim();
                out.push(if inner == "*" {
                    Segment::All
                } else if let Ok(n) = inner.parse::<usize>() {
                    Segment::Index(n)
                } else {
                    Segment::Key(inner.trim_matches(|c| c == '\'' || c == '"').to_string())
                });
                i = end + 1;
            }
            _ => {
                return Err(format!(
                    "{p}: expected . or [ at \"{}\"",
                    chars[i..].iter().collect::<String>()
                ));
            }
        }
    }
    Ok(out)
}

/// Calls `f` on every value at `path`; returns how many there were.
fn each_mut(
    value: &mut Value,
    path: &[Segment],
    f: &mut dyn FnMut(&mut Value) -> Result<(), String>,
) -> Result<usize, String> {
    let Some((first, rest)) = path.split_first() else {
        f(value)?;
        return Ok(1);
    };
    let mut count = 0;
    match (first, value) {
        (Segment::Key(k), Value::Object(map)) => {
            if let Some(v) = map.get_mut(k) {
                count += each_mut(v, rest, f)?;
            }
        }
        (Segment::Index(n), Value::Array(list)) => {
            if let Some(v) = list.get_mut(*n) {
                count += each_mut(v, rest, f)?;
            }
        }
        (Segment::All, Value::Array(list)) => {
            for v in list {
                count += each_mut(v, rest, f)?;
            }
        }
        (Segment::All, Value::Object(map)) => {
            for v in map.values_mut() {
                count += each_mut(v, rest, f)?;
            }
        }
        _ => {}
    }
    Ok(count)
}

/// Encrypts the fields at `paths` of a JSON `body` (comments already removed), or all of it for `$`, and returns the
/// new body text.
pub fn encrypt_body(body: &str, paths: &[String], cipher: &Cipher) -> Result<String, String> {
    if paths.iter().any(|p| p.trim() == "$") {
        return cipher.encrypt(body.as_bytes());
    }
    let mut value: Value = serde_json::from_str(body)
        .map_err(|e| format!("to encrypt fields the body must be JSON ({e})"))?;
    for path in paths {
        let segments = parse_path(path)?;
        let found = each_mut(&mut value, &segments, &mut |v| {
            let plain = match &*v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            *v = Value::String(cipher.encrypt(plain.as_bytes())?);
            Ok(())
        })?;
        if found == 0 {
            return Err(format!("the body has nothing at {path} to encrypt"));
        }
    }
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

/// What decrypting a response did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decryption {
    /// The body as it came, before decrypting.
    pub original: String,
    /// The paths that were decrypted, and how many values each had.
    pub fields: Vec<(String, usize)>,
}

/// Decrypts the fields at `paths` of a JSON response body, or all of it for `$` (a bare or JSON-quoted string), and
/// returns the new body.
pub fn decrypt_body(
    body: &str,
    paths: &[String],
    cipher: &Cipher,
) -> Result<(String, Vec<(String, usize)>), String> {
    // Text stays text ("4111" is a card number, not a number); objects and arrays come back as JSON.
    let plain_value = |bytes: Vec<u8>| -> Value {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if text.trim_start().starts_with(['{', '[']) {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        } else {
            Value::String(text)
        }
    };
    if paths.iter().any(|p| p.trim() == "$") {
        let text = serde_json::from_str::<String>(body).unwrap_or_else(|_| body.trim().to_string());
        let plain = cipher.decrypt(&text)?;
        let pretty = match plain_value(plain) {
            Value::String(s) => s,
            v => serde_json::to_string_pretty(&v).unwrap_or_default(),
        };
        return Ok((pretty, vec![("$".into(), 1)]));
    }
    let mut value: Value = serde_json::from_str(body)
        .map_err(|e| format!("to decrypt fields the response must be JSON ({e})"))?;
    let mut fields = Vec::new();
    for path in paths {
        let segments = parse_path(path)?;
        let found = each_mut(&mut value, &segments, &mut |v| {
            if let Value::String(s) = v {
                *v = plain_value(cipher.decrypt(s)?);
            }
            Ok(())
        })?;
        fields.push((path.clone(), found));
    }
    Ok((
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
        fields,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(algorithm: Algorithm) -> CryptoSettings {
        CryptoSettings {
            algorithm,
            ..CryptoSettings::default()
        }
    }

    const KEY_256: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY="; // "0123456789abcdef0123456789abcdef"
    const KEY_128: &str = "MDEyMzQ1Njc4OWFiY2RlZg=="; // "0123456789abcdef"

    #[test]
    fn every_algorithm_round_trips_with_random_ivs() {
        for (algorithm, key) in [
            (Algorithm::Aes256Gcm, KEY_256),
            (Algorithm::Aes128Gcm, KEY_128),
            (Algorithm::Aes256Cbc, KEY_256),
            (Algorithm::Aes128Cbc, KEY_128),
        ] {
            let cipher = Cipher::new(&settings(algorithm), key, "").unwrap();
            let a = cipher.encrypt(b"4111 1111 1111 1111").unwrap();
            let b = cipher.encrypt(b"4111 1111 1111 1111").unwrap();
            assert_ne!(a, b, "{algorithm:?}: a fresh IV each time");
            assert_eq!(cipher.decrypt(&a).unwrap(), b"4111 1111 1111 1111");
        }
    }

    #[test]
    fn known_cbc_vector_with_a_fixed_iv() {
        // From the cbc crate's own example: key 0x42 × 16, IV 0x24 × 16.
        let s = CryptoSettings {
            algorithm: Algorithm::Aes128Cbc,
            key_encoding: Encoding::Hex,
            output: Encoding::Hex,
            ..CryptoSettings::default()
        };
        let cipher = Cipher::new(&s, &"42".repeat(16), &"24".repeat(16)).unwrap();
        assert_eq!(
            cipher
                .encrypt(b"hello world! this is my plaintext.")
                .unwrap(),
            "c7fe247ef97b21f07cbdd26cb5d346bfd27867cb00d9486723e159978fb9a5f914cfb228a710de4171e396e7b6cf859e"
        );
    }

    #[test]
    fn fields_are_encrypted_and_decrypted_by_path() {
        let cipher = Cipher::new(&settings(Algorithm::Aes256Gcm), KEY_256, "").unwrap();
        let body = r#"{"name": "Ada", "card": {"number": "4111", "cvv": 123}, "items": [{"ssn": "1"}, {"ssn": "2"}]}"#;
        let paths = vec![
            "$.card.number".to_string(),
            "$.card.cvv".into(),
            "$.items[*].ssn".into(),
        ];
        let sealed = encrypt_body(body, &paths, &cipher).unwrap();
        let v: Value = serde_json::from_str(&sealed).unwrap();
        assert_eq!(v["name"], "Ada");
        assert!(v["card"]["number"].as_str().unwrap().len() > 20);
        assert!(v["card"]["cvv"].is_string());
        let (opened, fields) = decrypt_body(&sealed, &paths, &cipher).unwrap();
        let v: Value = serde_json::from_str(&opened).unwrap();
        assert_eq!(v["card"]["number"], "4111");
        assert_eq!(v["card"]["cvv"], "123", "decrypted values are text");
        assert_eq!(v["items"][1]["ssn"], "2");
        assert_eq!(fields[2], ("$.items[*].ssn".to_string(), 2));
        assert!(encrypt_body(body, &["$.missing".into()], &cipher).is_err());
    }

    #[test]
    fn whole_bodies_and_mistakes() {
        let cipher = Cipher::new(&settings(Algorithm::Aes256Gcm), KEY_256, "").unwrap();
        let sealed = encrypt_body(r#"{"a": 1}"#, &["$".into()], &cipher).unwrap();
        // A server may send it back as a JSON string.
        let (opened, _) = decrypt_body(&format!("\"{sealed}\""), &["$".into()], &cipher).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&opened).unwrap()["a"], 1);

        assert!(
            Cipher::new(&settings(Algorithm::Aes256Gcm), KEY_128, "")
                .unwrap_err()
                .contains("needs 32")
        );
        assert!(Cipher::new(&settings(Algorithm::Aes256Gcm), "", "").is_err());
        let other = Cipher::new(
            &settings(Algorithm::Aes256Gcm),
            &KEY_256.replace('M', "N"),
            "",
        )
        .unwrap();
        assert!(other.decrypt(&sealed).unwrap_err().contains("wrong key"));
        assert!(parse_path("card.number").is_err());
        assert_eq!(
            parse_path("$['a b'][2]").unwrap(),
            [Segment::Key("a b".into()), Segment::Index(2)]
        );
    }
}
