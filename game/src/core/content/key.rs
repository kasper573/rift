/// A fixed buffer so a key can be derived from a Rust identifier in `const` context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBuf {
    bytes: [u8; 64],
    len: usize,
}

impl KeyBuf {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("a key is ASCII")
    }

    pub const fn bytes(&self) -> &[u8] {
        self.bytes.split_at(self.len).0
    }
}

pub const fn key_of(ident: &str) -> KeyBuf {
    let source = ident.as_bytes();
    let mut bytes = [0u8; 64];
    let mut len = 0;
    let mut i = 0;
    while i < source.len() {
        let c = source[i];
        if c.is_ascii_uppercase() {
            if i > 0 {
                bytes[len] = b'_';
                len += 1;
            }
            bytes[len] = c.to_ascii_lowercase();
        } else {
            bytes[len] = c;
        }
        len += 1;
        i += 1;
    }
    KeyBuf { bytes, len }
}

pub const fn sorted_index(keys: &[KeyBuf], key: &KeyBuf) -> u32 {
    let mut before = 0;
    let mut i = 0;
    while i < keys.len() {
        if less(keys[i].bytes(), key.bytes()) {
            before += 1;
        }
        i += 1;
    }
    before
}

const fn less(a: &[u8], b: &[u8]) -> bool {
    let mut i = 0;
    while i < a.len() && i < b.len() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
        i += 1;
    }
    a.len() < b.len()
}
