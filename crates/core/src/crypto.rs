//! Encrypting parts of a request body before it is sent, and decrypting parts of the response, for APIs that exchange
//! encrypted fields (card numbers, PINs, personal data) or whole encrypted payloads.
//!
//! - AES in GCM (authenticated, recommended) or CBC (PKCS#7 padding) mode, with a 128- or 256-bit key from a variable.
//!   Each value gets a fresh random IV, written in front of the ciphertext (`iv ‖ ciphertext [‖ tag]`), unless a fixed
//!   IV is given.
//! - RSA-OAEP (SHA-1 or SHA-256, MGF1 with the same hash, no label), with a PEM public key for encrypting and a PEM
//!   private key for decrypting, as browser libraries such as node-forge do.
//! - Hybrid responses: `base64(RSA(key) : RSA(iv) : AES-CBC(data))`, where the AES key and IV come RSA-encrypted with
//!   the data and are unwrapped with the private key.
//!
//! Keys come from `{{variables}}`, so they can be secrets in the keychain. Fields are chosen by JSON path
//! (`$.card.number`, `$.items[*].ssn`); `$` is the whole body. A non-string value is encrypted as its JSON text. A
//! decrypted value comes back as a string, or as JSON when it is a JSON object or array.

use aes_gcm::aead::{Aead, KeyInit};
use base64::Engine;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use ring::rand::{SecureRandom, SystemRandom};
use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
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
    /// RSA-OAEP with SHA-1 and MGF1-SHA-1, as node-forge's `RSA-OAEP` with `md: sha1`.
    #[serde(rename = "rsa-oaep-sha1")]
    RsaOaepSha1,
    #[serde(rename = "rsa-oaep-sha256")]
    RsaOaepSha256,
}

impl Algorithm {
    fn key_len(self) -> usize {
        match self {
            Algorithm::Aes256Gcm | Algorithm::Aes256Cbc => 32,
            _ => 16,
        }
    }

    fn iv_len(self) -> usize {
        if self.gcm() { 12 } else { 16 }
    }

    fn gcm(self) -> bool {
        matches!(self, Algorithm::Aes256Gcm | Algorithm::Aes128Gcm)
    }

    pub fn is_rsa(self) -> bool {
        matches!(self, Algorithm::RsaOaepSha1 | Algorithm::RsaOaepSha256)
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

/// The hash of RSA-OAEP (and of its MGF1).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum OaepHash {
    #[default]
    Sha1,
    Sha256,
}

/// A response that carries its own AES key and IV, each encrypted with the RSA public key:
/// `key ‖ separator ‖ iv ‖ separator ‖ data`, the data AES-CBC (the key's length picks AES-128, -192 or -256).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct HybridFormat {
    pub separator: String,
    /// The joined parts are themselves base64 (the value is base64 of `key:iv:data`).
    pub wrapped: bool,
    /// The RSA-decrypted key and IV are base64 text, not raw bytes.
    pub keys_base64: bool,
    /// RSA-OAEP's hash for the key and IV.
    pub hash: OaepHash,
}

impl Default for HybridFormat {
    fn default() -> Self {
        Self {
            separator: ":".into(),
            wrapped: true,
            keys_base64: true,
            hash: OaepHash::Sha1,
        }
    }
}

/// A request's encryption settings, as written in its file. `key`, `iv` and `private_key` may use `{{variables}}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CryptoSettings {
    pub algorithm: Algorithm,
    /// The AES key, or for RSA the public key (PEM), such as `{{encryptionKey}}`.
    pub key: String,
    #[serde(skip_serializing_if = "is_base64")]
    pub key_encoding: Encoding,
    /// A fixed AES IV; empty means a random one for every value, written before the ciphertext.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub iv: String,
    /// How the ciphertext is written: base64 or hex.
    #[serde(skip_serializing_if = "is_base64")]
    pub output: Encoding,
    /// The RSA private key (PEM), such as `{{rsaPrivateKey}}`, to decrypt RSA-encrypted fields and hybrid responses.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub private_key: String,
    /// Response values that carry their own RSA-encrypted AES key and IV.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<HybridFormat>,
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

/// A PEM block as written, or squeezed onto one line (as `.env` files keep keys, sometimes with literal `\n`s),
/// rewrapped so it parses.
fn normalize_pem(text: &str) -> Result<String, String> {
    let text = text
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .replace("\\n", "\n");
    let begin = text
        .find("-----BEGIN ")
        .ok_or("the key is not a PEM block (-----BEGIN ...-----)")?;
    let label_end = text[begin + 11..]
        .find("-----")
        .ok_or("the key's BEGIN line is incomplete")?
        + begin
        + 11;
    let label = &text[begin + 11..label_end];
    let end_marker = format!("-----END {label}-----");
    let end = text
        .find(&end_marker)
        .ok_or_else(|| format!("the key has no {end_marker} line"))?;
    let body: String = text[label_end + 5..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();
    Ok(format!(
        "-----BEGIN {label}-----\n{}\n{end_marker}\n",
        lines.join("\n")
    ))
}

fn public_key(pem: &str) -> Result<RsaPublicKey, String> {
    let pem = normalize_pem(pem)?;
    if pem.contains("PRIVATE KEY") {
        return Err("this is a private key; encrypting needs the public key".into());
    }
    RsaPublicKey::from_public_key_pem(&pem)
        .or_else(|_| RsaPublicKey::from_pkcs1_pem(&pem))
        .map_err(|e| format!("not an RSA public key ({e})"))
}

fn private_key(pem: &str) -> Result<RsaPrivateKey, String> {
    let pem = normalize_pem(pem)?;
    if pem.contains("ENCRYPTED PRIVATE KEY") {
        return Err("the private key is protected with a passphrase; export it without one".into());
    }
    RsaPrivateKey::from_pkcs8_pem(&pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(&pem))
        .map_err(|e| format!("not an RSA private key ({e})"))
}

fn rsa_encrypt(key: &RsaPublicKey, hash: OaepHash, plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
    let result = match hash {
        OaepHash::Sha1 => key.encrypt(&mut rng, Oaep::<sha1::Sha1>::new(), plain),
        OaepHash::Sha256 => key.encrypt(&mut rng, Oaep::<rsa::sha2::Sha256>::new(), plain),
    };
    result.map_err(|e| match e {
        rsa::Error::MessageTooLong => format!(
            "{} bytes is too long to encrypt with this RSA key; RSA-OAEP takes short values such as PINs and keys",
            plain.len()
        ),
        e => format!("RSA encryption failed: {e}"),
    })
}

fn rsa_decrypt(key: &RsaPrivateKey, hash: OaepHash, data: &[u8]) -> Result<Vec<u8>, String> {
    match hash {
        OaepHash::Sha1 => key.decrypt(Oaep::<sha1::Sha1>::new(), data),
        OaepHash::Sha256 => key.decrypt(Oaep::<rsa::sha2::Sha256>::new(), data),
    }
    .map_err(|_| {
        format!(
            "could not decrypt with the RSA private key: a different key pair, hash (OAEP {}), or the value is not \
             RSA-encrypted",
            if hash == OaepHash::Sha1 { "SHA-1" } else { "SHA-256" }
        )
    })
}

/// AES-CBC with PKCS#7, the key's length choosing AES-128, -192 or -256.
fn aes_cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let wrong =
        || "could not decrypt the data: the unwrapped AES key and IV don't fit it".to_string();
    let bad = |e: cbc::cipher::InvalidLength| {
        format!("the unwrapped AES key or IV has the wrong length ({e})")
    };
    match key.len() {
        16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv)
            .map_err(bad)?
            .decrypt_padded_vec::<Pkcs7>(data)
            .map_err(|_| wrong()),
        24 => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, iv)
            .map_err(bad)?
            .decrypt_padded_vec::<Pkcs7>(data)
            .map_err(|_| wrong()),
        32 => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
            .map_err(bad)?
            .decrypt_padded_vec::<Pkcs7>(data)
            .map_err(|_| wrong()),
        n => Err(format!(
            "the unwrapped AES key is {n} bytes; AES needs 16, 24 or 32"
        )),
    }
}

/// AES with the settings' key.
struct Aes {
    algorithm: Algorithm,
    key: Vec<u8>,
    iv: Option<Vec<u8>>,
}

/// Settings with keys parsed and checked, once per request.
pub struct Cipher {
    algorithm: Algorithm,
    aes: Option<Aes>,
    public: Option<RsaPublicKey>,
    private: Option<RsaPrivateKey>,
    hybrid: Option<HybridFormat>,
    output: Encoding,
}

// Without the keys, so they never end up in a log.
impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cipher({})", name(self.algorithm))
    }
}

impl Cipher {
    /// `key`, `iv` and `private_key` are the settings' values after variables.
    pub fn new(settings: &CryptoSettings, key: &str, iv: &str) -> Result<Self, String> {
        Self::with_private_key(settings, key, iv, "")
    }

    pub fn with_private_key(
        settings: &CryptoSettings,
        key: &str,
        iv: &str,
        private: &str,
    ) -> Result<Self, String> {
        let algorithm = settings.algorithm;
        let output = if settings.output == Encoding::Hex {
            Encoding::Hex
        } else {
            Encoding::Base64
        };
        let private = if private.trim().is_empty() {
            None
        } else {
            Some(private_key(private)?)
        };
        if settings.hybrid.is_some() && private.is_none() && !settings.decrypt.is_empty() {
            return Err("hybrid responses need the RSA private key: set one (such as {{rsaPrivateKey}}) in the \
                        request's Crypto tab"
                .into());
        }
        let mut cipher = Self {
            algorithm,
            aes: None,
            public: None,
            private,
            hybrid: settings.hybrid.clone(),
            output,
        };
        if algorithm.is_rsa() {
            if !key.trim().is_empty() {
                cipher.public = Some(public_key(key)?);
            } else if !settings.encrypt.is_empty() {
                return Err("no RSA public key: set one (such as {{rsaPublicKey}}) in the request's Crypto tab".into());
            }
            if cipher.private.is_none() && cipher.hybrid.is_none() && !settings.decrypt.is_empty() {
                return Err("decrypting RSA-encrypted fields needs the RSA private key: set one in the Crypto tab".into());
            }
            return Ok(cipher);
        }
        // AES's own key, unless the request only decrypts hybrid responses (which bring their key).
        if settings.hybrid.is_some() && settings.encrypt.is_empty() && key.trim().is_empty() {
            return Ok(cipher);
        }
        if key.trim().is_empty() {
            return Err("no encryption key: set one (such as {{encryptionKey}}) in the request's Crypto tab".into());
        }
        let key = decode(key, settings.key_encoding, "key")?;
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
        cipher.aes = Some(Aes { algorithm, key, iv });
        Ok(cipher)
    }

    fn oaep(&self) -> OaepHash {
        if self.algorithm == Algorithm::RsaOaepSha256 {
            OaepHash::Sha256
        } else {
            OaepHash::Sha1
        }
    }

    pub fn encrypt(&self, plain: &[u8]) -> Result<String, String> {
        if self.algorithm.is_rsa() {
            let key = self
                .public
                .as_ref()
                .ok_or("no RSA public key to encrypt with")?;
            return Ok(encode(&rsa_encrypt(key, self.oaep(), plain)?, self.output));
        }
        let aes = self.aes.as_ref().ok_or("no AES key to encrypt with")?;
        let iv = match &aes.iv {
            Some(iv) => iv.clone(),
            None => {
                let mut iv = vec![0u8; aes.algorithm.iv_len()];
                SystemRandom::new()
                    .fill(&mut iv)
                    .map_err(|_| "no random numbers available".to_string())?;
                iv
            }
        };
        let sealed = match aes.algorithm {
            Algorithm::Aes256Gcm => aes_gcm::Aes256Gcm::new_from_slice(&aes.key)
                .map_err(|e| e.to_string())?
                .encrypt(iv.as_slice().try_into().map_err(|_| "bad IV")?, plain),
            Algorithm::Aes128Gcm => aes_gcm::Aes128Gcm::new_from_slice(&aes.key)
                .map_err(|e| e.to_string())?
                .encrypt(iv.as_slice().try_into().map_err(|_| "bad IV")?, plain),
            Algorithm::Aes256Cbc => Ok(cbc::Encryptor::<aes::Aes256>::new_from_slices(
                &aes.key, &iv,
            )
            .map_err(|e| e.to_string())?
            .encrypt_padded_vec::<Pkcs7>(plain)),
            _ => Ok(
                cbc::Encryptor::<aes::Aes128>::new_from_slices(&aes.key, &iv)
                    .map_err(|e| e.to_string())?
                    .encrypt_padded_vec::<Pkcs7>(plain),
            ),
        }
        .map_err(|_| "encryption failed".to_string())?;
        let out = if aes.iv.is_some() {
            sealed
        } else {
            [iv, sealed].concat()
        };
        Ok(encode(&out, self.output))
    }

    pub fn decrypt(&self, text: &str) -> Result<Vec<u8>, String> {
        if let Some(format) = &self.hybrid {
            return self.decrypt_hybrid(text, format);
        }
        if self.algorithm.is_rsa() {
            let key = self
                .private
                .as_ref()
                .ok_or("no RSA private key to decrypt with")?;
            return rsa_decrypt(
                key,
                self.oaep(),
                &decode(text, self.output, "value to decrypt")?,
            );
        }
        let aes = self.aes.as_ref().ok_or("no AES key to decrypt with")?;
        let bytes = decode(text, self.output, "value to decrypt")?;
        let (iv, data) = match &aes.iv {
            Some(iv) => (iv.as_slice(), bytes.as_slice()),
            None if bytes.len() > aes.algorithm.iv_len() => bytes.split_at(aes.algorithm.iv_len()),
            None => return Err("the value is too short to hold an IV and ciphertext".into()),
        };
        let wrong = || {
            format!(
                "could not decrypt: wrong key, IV, algorithm ({}) or encoding, or the value is not encrypted",
                name(aes.algorithm)
            )
        };
        match aes.algorithm {
            Algorithm::Aes256Gcm => aes_gcm::Aes256Gcm::new_from_slice(&aes.key)
                .map_err(|e| e.to_string())?
                .decrypt(iv.try_into().map_err(|_| wrong())?, data)
                .map_err(|_| wrong()),
            Algorithm::Aes128Gcm => aes_gcm::Aes128Gcm::new_from_slice(&aes.key)
                .map_err(|e| e.to_string())?
                .decrypt(iv.try_into().map_err(|_| wrong())?, data)
                .map_err(|_| wrong()),
            Algorithm::Aes256Cbc => cbc::Decryptor::<aes::Aes256>::new_from_slices(&aes.key, iv)
                .map_err(|e| e.to_string())?
                .decrypt_padded_vec::<Pkcs7>(data)
                .map_err(|_| wrong()),
            _ => cbc::Decryptor::<aes::Aes128>::new_from_slices(&aes.key, iv)
                .map_err(|e| e.to_string())?
                .decrypt_padded_vec::<Pkcs7>(data)
                .map_err(|_| wrong()),
        }
    }

    /// `base64(RSA(key) : RSA(iv) : AES-CBC(data))`, as `format` describes it.
    fn decrypt_hybrid(&self, value: &str, format: &HybridFormat) -> Result<Vec<u8>, String> {
        let private = self
            .private
            .as_ref()
            .ok_or("no RSA private key to unwrap the AES key with")?;
        let joined = if format.wrapped {
            String::from_utf8(decode(value, Encoding::Base64, "response")?).map_err(|_| {
                "the response's base64 does not hold text: turn off \"wrapped\"".to_string()
            })?
        } else {
            value.trim().to_string()
        };
        let separator = if format.separator.is_empty() {
            ":"
        } else {
            &format.separator
        };
        let parts: Vec<&str> = joined.split(separator).collect();
        if parts.len() != 3 {
            return Err(format!(
                "expected three parts, key{separator}iv{separator}data, but found {}",
                parts.len()
            ));
        }
        let unwrap = |part: &str, what: &str| -> Result<Vec<u8>, String> {
            let plain = rsa_decrypt(private, format.hash, &decode(part, Encoding::Base64, what)?)?;
            if format.keys_base64 {
                decode(&String::from_utf8_lossy(&plain), Encoding::Base64, what)
            } else {
                Ok(plain)
            }
        };
        let key = unwrap(parts[0], "AES key")?;
        let iv = unwrap(parts[1], "IV")?;
        aes_cbc_decrypt(&key, &iv, &decode(parts[2], Encoding::Base64, "data")?)
    }
}

fn name(a: Algorithm) -> &'static str {
    match a {
        Algorithm::Aes256Gcm => "AES-256-GCM",
        Algorithm::Aes128Gcm => "AES-128-GCM",
        Algorithm::Aes256Cbc => "AES-256-CBC",
        Algorithm::Aes128Cbc => "AES-128-CBC",
        Algorithm::RsaOaepSha1 => "RSA-OAEP (SHA-1)",
        Algorithm::RsaOaepSha256 => "RSA-OAEP (SHA-256)",
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
