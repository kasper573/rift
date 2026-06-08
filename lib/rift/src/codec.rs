use std::any::type_name;

// `Send` so worlds (whose columns store `Wire` values) can move to worker threads — the cluster
// ticks shards in parallel. Every wire type is plain data, so this is always satisfied.
pub trait Wire: Sized + Send {
    fn encode(&self, out: &mut Vec<u8>);
    fn decode(bytes: &mut &[u8]) -> Option<Self>;
}

macro_rules! primitive {
    ($($number:ty),*) => {$(
        impl Wire for $number {
            fn encode(&self, out: &mut Vec<u8>) { out.extend_from_slice(&self.to_le_bytes()); }
            fn decode(bytes: &mut &[u8]) -> Option<Self> {
                let (head, rest) = bytes.split_at_checked(std::mem::size_of::<$number>())?;
                *bytes = rest;
                Some(<$number>::from_le_bytes(head.try_into().ok()?))
            }
        }
    )*};
}
primitive!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64);

impl Wire for bool {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        Some(u8::decode(bytes)? != 0)
    }
}

impl Wire for String {
    fn encode(&self, out: &mut Vec<u8>) {
        (self.len() as u32).encode(out);
        out.extend_from_slice(self.as_bytes());
    }
    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        let length = u32::decode(bytes)? as usize;
        let (head, rest) = bytes.split_at_checked(length)?;
        *bytes = rest;
        String::from_utf8(head.to_vec()).ok()
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        (self.len() as u32).encode(out);
        for item in self {
            item.encode(out);
        }
    }
    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        let length = u32::decode(bytes)? as usize;
        let mut values = Vec::with_capacity(length.min(4096));
        for _ in 0..length {
            values.push(T::decode(bytes)?);
        }
        Some(values)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Some(value) => {
                true.encode(out);
                value.encode(out);
            }
            None => false.encode(out),
        }
    }
    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        Some(if bool::decode(bytes)? {
            Some(T::decode(bytes)?)
        } else {
            None
        })
    }
}

// LEB128 varint for lengths/counts: one byte for values < 128 (the common case for our small
// components and short delta lists), growing 7 bits at a time.
pub(crate) fn write_varint(mut value: usize, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}
pub(crate) fn read_varint(bytes: &mut &[u8]) -> Option<usize> {
    let mut result = 0usize;
    let mut shift = 0u32;
    loop {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        result |= ((byte & 0x7f) as usize).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

pub(crate) fn tag<T: 'static>() -> Tag {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in type_name::<T>().as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Tag(hash)
}

#[derive(crate::Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub(crate) struct Tag(pub u64);

#[derive(crate::Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub(crate) struct Opcode(pub u8);

pub const SNAPSHOT: Opcode = Opcode(0);
pub const EVENT: Opcode = Opcode(1);
