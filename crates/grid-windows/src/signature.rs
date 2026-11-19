//! Checking that an update really comes from Grid's author. Each release's `grid.exe` is signed
//! (ECDSA P-256 over SHA-256, the signature uploaded as `grid.exe.sig`) with a key only the
//! author has, kept offline (`tools/sign-release.ps1`). Grid checks the signature against the
//! key's public half, built in from `release-key.hex` (written by `tools/new-signing-key.ps1`),
//! using Windows' own cryptography (bcrypt). So an update can't be swapped for something else,
//! even by someone who got into the GitHub account.

use std::ptr::null_mut;

use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_ALG_HANDLE, BCRYPT_ECCKEY_BLOB, BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALGORITHM,
    BCRYPT_ECDSA_PUBLIC_P256_MAGIC, BCRYPT_KEY_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCloseAlgorithmProvider,
    BCryptDestroyKey, BCryptHash, BCryptImportKeyPair, BCryptOpenAlgorithmProvider, BCryptVerifySignature,
};

/// The release key's public half: X then Y, 32 bytes each.
pub const RELEASE_KEY: [u8; 64] = hex64(include_str!("../release-key.hex"));

/// Length of a signature: R then S, 32 bytes each.
pub const SIGNATURE_LEN: usize = 64;

/// Whether `signature` is a valid signature of `data` by the private half of `key`.
pub fn verify(key: &[u8; 64], data: &[u8], signature: &[u8]) -> bool {
    if signature.len() != SIGNATURE_LEN {
        return false;
    }
    let mut hash = [0u8; 32];
    // SAFETY: every pointer and length passed below belongs to a live local buffer.
    unsafe {
        let ok = BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null_mut(), 0, data.as_ptr(), data.len() as u32, hash.as_mut_ptr(), 32);
        if ok != 0 {
            return false;
        }
        let mut alg: BCRYPT_ALG_HANDLE = null_mut();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_ECDSA_P256_ALGORITHM, std::ptr::null(), 0) != 0 {
            return false;
        }
        // The public key blob: a header, then X and Y.
        let header = BCRYPT_ECCKEY_BLOB { dwMagic: BCRYPT_ECDSA_PUBLIC_P256_MAGIC, cbKey: 32 };
        let mut blob = Vec::with_capacity(size_of::<BCRYPT_ECCKEY_BLOB>() + 64);
        blob.extend_from_slice(&header.dwMagic.to_le_bytes());
        blob.extend_from_slice(&header.cbKey.to_le_bytes());
        blob.extend_from_slice(key);
        let mut handle: BCRYPT_KEY_HANDLE = null_mut();
        let imported = BCryptImportKeyPair(alg, null_mut(), BCRYPT_ECCPUBLIC_BLOB, &mut handle, blob.as_ptr(), blob.len() as u32, 0) == 0;
        let valid = imported
            && BCryptVerifySignature(handle, std::ptr::null(), hash.as_ptr(), 32, signature.as_ptr(), SIGNATURE_LEN as u32, 0) == 0;
        if imported {
            BCryptDestroyKey(handle);
        }
        BCryptCloseAlgorithmProvider(alg, 0);
        valid
    }
}

/// Whether Windows accepts `key` as a P-256 public key.
#[cfg(test)]
fn key_loads(key: &[u8; 64]) -> bool {
    let mut blob = Vec::with_capacity(8 + 64);
    blob.extend_from_slice(&BCRYPT_ECDSA_PUBLIC_P256_MAGIC.to_le_bytes());
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(key);
    // SAFETY: as in `verify`.
    unsafe {
        let mut alg: BCRYPT_ALG_HANDLE = null_mut();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_ECDSA_P256_ALGORITHM, std::ptr::null(), 0) != 0 {
            return false;
        }
        let mut handle: BCRYPT_KEY_HANDLE = null_mut();
        let ok = BCryptImportKeyPair(alg, null_mut(), BCRYPT_ECCPUBLIC_BLOB, &mut handle, blob.as_ptr(), blob.len() as u32, 0) == 0;
        if ok {
            BCryptDestroyKey(handle);
        }
        BCryptCloseAlgorithmProvider(alg, 0);
        ok
    }
}

/// 128 hex digits (whitespace around them ignored) as 64 bytes, at compile time: a missing or
/// mistyped key stops the build rather than shipping a Grid that can't check updates.
const fn hex64(text: &str) -> [u8; 64] {
    let bytes = text.as_bytes();
    let mut out = [0u8; 64];
    let (mut i, mut n) = (0, 0);
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => panic!("release-key.hex: not a hex digit"),
        };
        assert!(n < 128, "release-key.hex: more than 64 bytes");
        out[n / 2] |= if n % 2 == 0 { digit << 4 } else { digit };
        n += 1;
    }
    assert!(n == 128, "release-key.hex: needs 128 hex digits (64 bytes)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key, made for these tests only (not the release key), and its signature of
    // MESSAGE, made with .NET's ECDsa (the same kind of signature tools/sign-release.ps1 makes).
    const KEY: [u8; 64] = hex64(
        "B5B887AD4BAD9D5198CDD271E5F2DC232008D801800035D1CEA60AF412D4E560\
         D5C8063CF6B0CAAA87CB91860EB01DFEDEF4C5A96E370BEC7A6972F48C2F3444",
    );
    const SIGNATURE: [u8; 64] = hex64(
        "873205577D6BF7DC0D2D4441A6BFB6B28D8D7BB6776EFE60E7DC1899425D3248\
         913D5D447389EDBEECE192B26D085F0616A36EE5D229AE0F486D1BD21E78E391",
    );
    const MESSAGE: &[u8] = b"Grid test update";

    #[test]
    fn release_key_is_a_valid_key() {
        // A wrong release-key.hex would make every update fail its check.
        assert!(key_loads(&RELEASE_KEY));
        assert!(!key_loads(&[0u8; 64]));
    }

    #[test]
    fn accepts_a_valid_signature() {
        assert!(verify(&KEY, MESSAGE, &SIGNATURE));
    }

    #[test]
    fn rejects_changed_data_signature_or_key() {
        assert!(!verify(&KEY, b"Grid test updatE", &SIGNATURE));
        let mut sig = SIGNATURE;
        sig[10] ^= 1;
        assert!(!verify(&KEY, MESSAGE, &sig));
        assert!(!verify(&KEY, MESSAGE, &SIGNATURE[..63]));
        let mut key = KEY;
        key[0] ^= 1;
        assert!(!verify(&key, MESSAGE, &SIGNATURE));
    }
}
