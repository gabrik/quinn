use std::sync::Arc;
use std::time::Duration;

use rand::RngCore;

use crate::shared::ConnectionId;
use crate::{crypto, MAX_CID_SIZE};

/// Generates connection IDs for incoming connections
pub trait ConnectionIdGenerator: Send {
    /// Generates a new CID
    ///
    /// Connection IDs MUST NOT contain any information that can be used by
    /// an external observer (that is, one that does not cooperate with the
    /// issuer) to correlate them with other connection IDs for the same
    /// connection.
    fn generate_cid(&mut self) -> ConnectionId;
    /// Returns the length of a CID for connections created by this generator
    fn cid_len(&self) -> usize;
    /// Returns the lifetime of generated Connection IDs
    ///
    /// Connection IDs will be retired after the returned `Duration`, if any. Assumed to be constant.
    fn cid_lifetime(&self) -> Option<Duration>;

    /// Quickly determine whether `cid` could have been generated by this generator
    ///
    /// False positives are permitted, but will reduce the efficiency with which invalid packets are
    /// discarded.
    fn validate(&self, _cid: &ConnectionId) -> Result<(), InvalidCid> {
        Ok(())
    }
}

/// The connection ID was not recognized by the [`ConnectionIdGenerator`]
#[derive(Debug, Copy, Clone)]
pub struct InvalidCid;

/// Generates purely random connection IDs of a specified length
///
/// Random CIDs can be smaller than those produced by [`KeyedConnectionIdGenerator`], but cannot be
/// usefully [`validate`](ConnectionIdGenerator::validate)d.
#[derive(Debug, Clone, Copy)]
pub struct RandomConnectionIdGenerator {
    cid_len: usize,
    lifetime: Option<Duration>,
}

impl Default for RandomConnectionIdGenerator {
    fn default() -> Self {
        Self {
            cid_len: 8,
            lifetime: None,
        }
    }
}

impl RandomConnectionIdGenerator {
    /// Initialize Random CID generator with a fixed CID length
    ///
    /// The given length must be less than or equal to MAX_CID_SIZE.
    pub fn new(cid_len: usize) -> Self {
        debug_assert!(cid_len <= MAX_CID_SIZE);
        Self {
            cid_len,
            ..Self::default()
        }
    }

    /// Set the lifetime of CIDs created by this generator
    pub fn set_lifetime(&mut self, d: Duration) -> &mut Self {
        self.lifetime = Some(d);
        self
    }
}

impl ConnectionIdGenerator for RandomConnectionIdGenerator {
    fn generate_cid(&mut self) -> ConnectionId {
        let mut bytes_arr = [0; MAX_CID_SIZE];
        rand::thread_rng().fill_bytes(&mut bytes_arr[..self.cid_len]);

        ConnectionId::new(&bytes_arr[..self.cid_len])
    }

    /// Provide the length of dst_cid in short header packet
    fn cid_len(&self) -> usize {
        self.cid_len
    }

    fn cid_lifetime(&self) -> Option<Duration> {
        self.lifetime
    }
}

/// Generates 8-byte connection IDs that can be efficiently
/// [`validate`](ConnectionIdGenerator::validate)d
pub struct KeyedConnectionIdGenerator {
    key: Arc<dyn crypto::HmacKey>,
    lifetime: Option<Duration>,
}

impl KeyedConnectionIdGenerator {
    /// Create a generator with a random key
    #[cfg(feature = "ring")]
    pub fn new() -> Self {
        let mut reset_key = [0; 64];
        rand::thread_rng().fill_bytes(&mut reset_key);

        let key = Arc::new(ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &reset_key));
        Self::from_key(key)
    }

    /// Create a generator with a specific key
    pub fn from_key(key: Arc<dyn crypto::HmacKey>) -> Self {
        assert!(
            key.signature_len() < MAX_SIGNATURE_LEN,
            "key must generate at most a 128 byte signature"
        );
        Self {
            key,
            lifetime: None,
        }
    }

    /// Set the lifetime of CIDs created by this generator
    pub fn set_lifetime(&mut self, d: Duration) -> &mut Self {
        self.lifetime = Some(d);
        self
    }
}

#[cfg(feature = "ring")]
impl Default for KeyedConnectionIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionIdGenerator for KeyedConnectionIdGenerator {
    fn generate_cid(&mut self) -> ConnectionId {
        let mut bytes_arr = [0; NONCE_LEN + MAX_SIGNATURE_LEN];
        rand::thread_rng().fill_bytes(&mut bytes_arr[..NONCE_LEN]);
        let (nonce, signature) = bytes_arr.split_at_mut(NONCE_LEN);
        self.key
            .sign(nonce, &mut signature[..self.key.signature_len()]);
        ConnectionId::new(&bytes_arr[..self.cid_len()])
    }

    fn cid_len(&self) -> usize {
        NONCE_LEN + SIGNATURE_LEN
    }

    fn cid_lifetime(&self) -> Option<Duration> {
        self.lifetime
    }

    fn validate(&self, cid: &ConnectionId) -> Result<(), InvalidCid> {
        let (nonce, signature) = cid.split_at(NONCE_LEN);
        let mut expected_signature = [0; MAX_SIGNATURE_LEN];
        self.key
            .sign(nonce, &mut expected_signature[..self.key.signature_len()]);
        (expected_signature[..SIGNATURE_LEN] == signature[..])
            .then_some(())
            .ok_or(InvalidCid)
    }
}

const NONCE_LEN: usize = 3; // Good for more than 16 million connections
const SIGNATURE_LEN: usize = 5; // 8-byte total CID length
const MAX_SIGNATURE_LEN: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "ring")]
    fn validate_keyed_cid() {
        let mut generator = KeyedConnectionIdGenerator::new();
        let cid = generator.generate_cid();
        generator.validate(&cid).unwrap();
    }
}
