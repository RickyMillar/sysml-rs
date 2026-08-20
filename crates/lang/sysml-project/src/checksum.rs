use sha2::{Digest, Sha256};

/// Supported checksum algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Sha256,
}

impl ChecksumAlgorithm {
    /// Parse an algorithm name (case-insensitive).
    pub fn from_name(name: &str) -> crate::Result<Self> {
        match name.to_uppercase().as_str() {
            "SHA256" | "SHA-256" => Ok(Self::Sha256),
            other => Err(crate::Error::UnsupportedAlgorithm(other.to_owned())),
        }
    }

    /// Algorithm name as used in `.meta.json`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sha256 => "SHA256",
        }
    }
}

/// Compute a hex-encoded checksum of the given data.
pub fn compute_checksum(data: &[u8], algorithm: ChecksumAlgorithm) -> String {
    match algorithm {
        ChecksumAlgorithm::Sha256 => {
            let digest = Sha256::digest(data);
            hex::encode(digest)
        }
    }
}

/// Verify that `data` matches an expected hex-encoded checksum.
pub fn verify_checksum(
    data: &[u8],
    expected: &str,
    algorithm: ChecksumAlgorithm,
) -> crate::Result<bool> {
    let actual = compute_checksum(data, algorithm);
    Ok(actual == expected)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty() {
        let hash = compute_checksum(b"", ChecksumAlgorithm::Sha256);
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hello() {
        let hash = compute_checksum(b"hello", ChecksumAlgorithm::Sha256);
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn verify_matches() {
        let expected = compute_checksum(b"test data", ChecksumAlgorithm::Sha256);
        assert!(verify_checksum(b"test data", &expected, ChecksumAlgorithm::Sha256).unwrap());
    }

    #[test]
    fn verify_mismatch() {
        assert!(
            !verify_checksum(b"test data", "0000000000000000", ChecksumAlgorithm::Sha256).unwrap()
        );
    }

    #[test]
    fn parse_algorithm_name() {
        assert_eq!(
            ChecksumAlgorithm::from_name("SHA256").unwrap(),
            ChecksumAlgorithm::Sha256
        );
        assert_eq!(
            ChecksumAlgorithm::from_name("sha-256").unwrap(),
            ChecksumAlgorithm::Sha256
        );
        assert!(ChecksumAlgorithm::from_name("MD5").is_err());
    }
}
