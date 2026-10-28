//! The PIN of the app lock.
//!
//! WhatsApp locks the app with the biometrics or the PIN of the phone, and
//! the desktop keeps the same idea with a PIN of its own: the window covers
//! everything until the PIN is written. The PIN never touches the disk: the
//! settings keep a random salt and the PBKDF2-SHA256 of the PIN, so a copy of
//! the file cannot be opened without the PIN.
//!
//! The lock is local to this profile. It does not change anything of the
//! linked device and it is not a second factor of the WhatsApp account.

use sha2::Sha256;

/// The rounds of the PBKDF2: a PIN of six digits is a small space, and every
/// extra round makes the guessing of a stolen file slower.
const ITERATIONS: u32 = 120_000;

/// The shortest PIN and the longest one that the keypad accepts.
pub(crate) const MIN_PIN_LENGTH: usize = 4;
pub(crate) const MAX_PIN_LENGTH: usize = 8;

/// The size of the key of the hash, in bytes.
const KEY_BYTES: usize = 32;

/// Whether the text is a PIN: digits only, between four and eight.
pub(crate) fn is_valid_pin(pin: &str) -> bool {
    let length = pin.chars().count();
    (MIN_PIN_LENGTH..=MAX_PIN_LENGTH).contains(&length)
        && pin.chars().all(|character| character.is_ascii_digit())
}

/// A salt for a new PIN, as the hex of the 128 bits of a UUID.
pub(crate) fn new_salt() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The PBKDF2-SHA256 of the PIN with the salt, as lowercase hex.
pub(crate) fn hash_pin(pin: &str, salt: &str) -> String {
    let mut key = [0u8; KEY_BYTES];
    pbkdf2::pbkdf2_hmac::<Sha256>(pin.as_bytes(), salt.as_bytes(), ITERATIONS, &mut key);
    let mut hash = String::with_capacity(KEY_BYTES * 2);
    for byte in key {
        use std::fmt::Write as _;
        let _ = write!(hash, "{byte:02x}");
    }
    hash
}

/// Whether the PIN opens the lock that the salt and the hash belong to.
pub(crate) fn verify(pin: &str, salt: &str, hash: &str) -> bool {
    if salt.is_empty() || hash.is_empty() || !is_valid_pin(pin) {
        return false;
    }
    let candidate = hash_pin(pin, salt);
    // The comparison is over the same length and does not stop at the first
    // different byte: the timing does not say how much of the hash matched.
    candidate.len() == hash.len()
        && candidate
            .bytes()
            .zip(hash.bytes())
            .fold(0u8, |difference, (left, right)| difference | (left ^ right))
            == 0
}

#[cfg(test)]
mod tests {
    use super::{hash_pin, is_valid_pin, new_salt, verify};

    #[test]
    fn a_pin_is_four_to_eight_digits() {
        assert!(is_valid_pin("1234"));
        assert!(is_valid_pin("12345678"));
        assert!(!is_valid_pin("123"));
        assert!(!is_valid_pin("123456789"));
        assert!(!is_valid_pin("12a4"));
        assert!(!is_valid_pin(""));
    }

    #[test]
    fn the_hash_changes_with_the_salt_and_the_pin() {
        let salt = new_salt();
        let hash = hash_pin("1234", &salt);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, hash_pin("1234", &salt));
        assert_ne!(hash, hash_pin("1234", "another-salt"));
        assert_ne!(hash, hash_pin("4321", &salt));
    }

    #[test]
    fn only_the_right_pin_opens_the_lock() {
        let salt = new_salt();
        let hash = hash_pin("2468", &salt);
        assert!(verify("2468", &salt, &hash));
        assert!(!verify("2469", &salt, &hash));
        assert!(!verify("2468", "", &hash));
        assert!(!verify("2468", &salt, ""));
    }
}
