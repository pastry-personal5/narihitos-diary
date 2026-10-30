use diary_core::Digest;

const BLOCK_BYTES: usize = 64;
const TAG_HEX_BYTES: usize = 8;

/// An opaque, fixed-width correlation reference for a resource (a workflow
/// day or an assistant backend). Only [`DiagnosticTagger::tag`] can build
/// one, so every instance is eight lowercase hex digits and is safe to write
/// into an `ndlog-v1` record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticTag([u8; TAG_HEX_BYTES]);

impl DiagnosticTag {
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("00000000")
    }
}

/// Derives stable per-resource tags with HMAC-SHA256 under a private,
/// persistent key. A plain SHA-256 of a date or source name would be
/// brute-forceable by anyone reading the log, because the input space is tiny;
/// the keyed MAC is not, while the same resource still maps to the same tag
/// across records and restarts.
pub struct DiagnosticTagger {
    key: [u8; 32],
}

impl DiagnosticTagger {
    #[must_use]
    pub const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// `domain` separates resource kinds so equal values in different kinds
    /// never share a tag.
    #[must_use]
    pub fn tag(&self, domain: &'static str, value: &[u8]) -> DiagnosticTag {
        let mut message = Vec::with_capacity(domain.len() + 1 + value.len());
        message.extend_from_slice(domain.as_bytes());
        message.push(0);
        message.extend_from_slice(value);
        let mac = hmac_sha256(&self.key, &message);
        let mut hex = [0_u8; TAG_HEX_BYTES];
        let (pairs, _) = hex.as_chunks_mut::<2>();
        for (pair, byte) in pairs.iter_mut().zip(mac) {
            pair[0] = HEX[usize::from(byte >> 4)];
            pair[1] = HEX[usize::from(byte & 0x0f)];
        }
        DiagnosticTag(hex)
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn sha256(input: &[u8]) -> [u8; 32] {
    let digest = Digest::of_bytes(input);
    let mut out = [0_u8; 32];
    let (pairs, _) = digest.as_str().as_bytes().as_chunks::<2>();
    for (slot, pair) in out.iter_mut().zip(pairs) {
        *slot = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    out
}

/// `Digest` always renders lowercase hex, so no other byte can occur.
const fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

pub(crate) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        block[..32].copy_from_slice(&sha256(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(BLOCK_BYTES + message.len());
    inner.extend(block.iter().map(|byte| byte ^ 0x36));
    inner.extend_from_slice(message);
    let inner_digest = sha256(&inner);
    let mut outer = Vec::with_capacity(BLOCK_BYTES + 32);
    outer.extend(block.iter().map(|byte| byte ^ 0x5c));
    outer.extend_from_slice(&inner_digest);
    sha256(&outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn hmac_matches_rfc_4231_vectors() {
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Key longer than one block is hashed first (RFC 4231 case 6).
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn tags_are_stable_keyed_and_domain_separated() {
        let tagger = DiagnosticTagger::new([7; 32]);
        let other = DiagnosticTagger::new([8; 32]);
        let day = tagger.tag("workflow", b"2026-10-30");
        assert_eq!(day, tagger.tag("workflow", b"2026-10-30"));
        assert_ne!(day, tagger.tag("workflow", b"2026-10-31"));
        assert_ne!(day, tagger.tag("backend", b"2026-10-30"));
        assert_ne!(day, other.tag("workflow", b"2026-10-30"));
        assert_eq!(day.as_str().len(), 8);
        assert!(day.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
