//! An SSH public key typed after `key-string` in `ip ssh pubkey-chain`, as IOS keeps it: the
//! `key-hash` line it prints in the key's place.

use base64::Engine as _;
use md5::{Digest, Md5};

/// A key as `key-hash` keeps it: the type its blob names, the MD5 of the blob in upper hex, and
/// the words after the blob as its comment.
#[derive(Debug, PartialEq)]
pub struct Key {
    pub kind: String,
    pub hex: String,
    pub comment: Option<String>,
}

/// The key a key's text makes, its lines joined as typed. The lab C9300 (17.6.4) printed
/// `key-hash ssh-rsa F7D30FD3AC9B60E00F3E2A65685F80D5` for its key typed with or without the
/// leading `ssh-rsa`, kept `two words here` after it as the comment, and could not decode a key
/// whose `ssh-rsa` stood on a line of its own. None for text with no blob it can decode.
pub fn key_hash(text: &str) -> Option<Key> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (at, kind, blob) = words.iter().enumerate().find_map(|(i, w)| {
        let blob = base64::engine::general_purpose::STANDARD.decode(w).ok()?;
        Some((i, key_type(&blob)?, blob))
    })?;
    let hex = Md5::digest(&blob)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    let comment = (at + 1 < words.len()).then(|| words[at + 1..].join(" "));
    Some(Key { kind, hex, comment })
}

/// The key type an SSH wire-format blob names: length-prefixed fields that fill the blob exactly,
/// the name first and the key's own fields after it (an RSA key's exponent and modulus).
fn key_type(blob: &[u8]) -> Option<String> {
    let mut fields = vec![];
    let mut rest = blob;
    while !rest.is_empty() {
        let len = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        fields.push(rest.get(4..4 + len)?);
        rest = &rest[4 + len..];
    }
    let name = std::str::from_utf8(fields.first()?).ok()?;
    (fields.len() >= 3 && !name.is_empty() && name.bytes().all(|b| b.is_ascii_graphic()))
        .then(|| name.to_string())
}

// the lab's key: RSA 2048, made for the lab C9300 (17.6.4)
#[cfg(test)]
pub const LAB_KEY: &str = "AAAAB3NzaC1yc2EAAAADAQABAAABAQCe8UFnGdoaDo2sLUEC5dN5Ixi38iCBL4q2fJ8G72Z+cKvJL/QprpCWFVzhuMIKVC0E+4CynqVaD11o/GFnUQE+yBrKzmhBnJ39faIuAVPQgnt/OXjrHpX60a4i856cJiAuwMIKkWgrgC73EG7nbH1LB9H1hOvG0TQ1T+/U8O7Gop2EU0LsAbiYMNTtO7q15DuL1CEteZkQfWcwoqqmV7g5iWCW/sFC06bU54nVdHH5J7NgA5WJ9396tgrn+p650qyjvEi29Pm6/cquzkJoTFrXsqHN0XsPjtrNH4c08+wyY4gfvLMOyYbnqncmAWGPqHvteIO+e+h1n7IuiNlygSYP";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_hashes_as_the_lab_c9300_printed_it() {
        let key = |comment: Option<&str>| Key {
            kind: "ssh-rsa".into(),
            hex: "F7D30FD3AC9B60E00F3E2A65685F80D5".into(),
            comment: comment.map(str::to_string),
        };
        assert_eq!(key_hash(LAB_KEY), Some(key(None)));
        assert_eq!(key_hash(&format!("ssh-rsa {LAB_KEY}")), Some(key(None)));
        assert_eq!(
            key_hash(&format!("ssh-rsa {LAB_KEY} two words here")),
            Some(key(Some("two words here")))
        );
        // `ssh-rsa` on a line of its own joins the blob
        assert_eq!(key_hash(&format!("ssh-rsa{LAB_KEY}")), None);
    }

    #[test]
    fn text_with_no_key_blob_makes_no_hash() {
        assert_eq!(key_hash(""), None);
        assert_eq!(key_hash("ssh-rsa"), None);
        // base64 that is no SSH key
        assert_eq!(key_hash("aGVsbG8gd29ybGQ="), None);
        // a blob cut short
        assert_eq!(key_hash(&LAB_KEY[..40]), None);
    }
}
