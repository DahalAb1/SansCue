use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Clone, Default)]
pub struct Security {
    pub origin: Option<String>,
    pub insecure_loopback: bool,
    pub operator_hash: Option<Vec<u8>>,
    pub operator_epoch: Option<String>,
    pub hmac_key: Option<Vec<u8>>,
    pub bee_credential: Option<String>,
    pub topics_credential: Option<String>,
}
impl Security {
    pub fn from_env() -> Result<Self, &'static str> {
        let get = |k| std::env::var(k).ok().filter(|v| !v.is_empty());
        let s = Self {
            origin: get("SESSIONS_PUBLIC_ORIGIN"),
            insecure_loopback: get("SESSIONS_ALLOW_INSECURE_LOOPBACK").as_deref() == Some("true"),
            operator_hash: decode_key(get("SESSIONS_OPERATOR_TOKEN_SHA256"))?,
            operator_epoch: get("SESSIONS_OPERATOR_EPOCH"),
            hmac_key: decode_key(get("SESSIONS_IDEMPOTENCY_HMAC_KEY"))?,
            bee_credential: get("SESSIONS_BEE_SERVICE_TOKEN"),
            topics_credential: get("SESSIONS_TOPICS_SERVICE_TOKEN"),
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(origin) = &self.origin {
            let u = url::Url::parse(origin).map_err(|_| "invalid public origin")?;
            if u.origin().ascii_serialization() != *origin {
                return Err("public origin must be canonical origin only");
            }
            let loopback = match u.host() {
                Some(url::Host::Domain("localhost")) => true,
                Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                _ => false,
            };
            if u.scheme() != "https"
                && !(u.scheme() == "http" && loopback && self.insecure_loopback)
            {
                return Err("HTTP requires explicit loopback development");
            }
        }
        Ok(())
    }
    pub fn operator_enabled(&self) -> bool {
        self.operator_hash.is_some() && self.operator_epoch.is_some() && self.hmac_key.is_some()
    }
    pub fn fingerprint(&self, bytes: &[u8], login: bool) -> Vec<u8> {
        if login {
            let mut mac = Hmac::<Sha256>::new_from_slice(self.hmac_key.as_deref().unwrap_or(&[]))
                .expect("HMAC accepts any key size");
            mac.update(bytes);
            mac.finalize().into_bytes().to_vec()
        } else {
            hash(bytes)
        }
    }
}
pub fn hash(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}
pub fn equal(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}
pub fn token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn decode_key(value: Option<String>) -> Result<Option<Vec<u8>>, &'static str> {
    value
        .map(|v| {
            hex::decode(v)
                .ok()
                .filter(|v| v.len() == 32)
                .ok_or("security key must be 32-byte hex")
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(value: &str, insecure: bool) -> Result<(), &'static str> {
        Security {
            origin: Some(value.into()),
            insecure_loopback: insecure,
            ..Security::default()
        }
        .validate()
    }

    #[test]
    fn http_cookies_require_explicit_loopback() {
        assert!(origin("http://localhost:5173", true).is_ok());
        assert!(origin("http://127.0.0.1:3000", true).is_ok());
        assert!(origin("http://[::1]:5173", true).is_ok());
        assert!(origin("http://localhost:5173", false).is_err());
        assert!(origin("http://192.168.1.10:5173", true).is_err());
        assert!(origin("https://rooms.example", false).is_ok());
        assert!(origin("https://rooms.example/join", false).is_err());
        assert!(origin("https://rooms.example/", false).is_err());
    }

    #[test]
    fn security_keys_are_32_byte_hex_or_absent() {
        assert!(decode_key(None).unwrap().is_none());
        assert!(decode_key(Some("abcd".into())).is_err());
        assert_eq!(
            decode_key(Some("ab".repeat(32))).unwrap().unwrap().len(),
            32
        );
        let disabled = Security::default();
        assert!(!disabled.operator_enabled());
    }
}
