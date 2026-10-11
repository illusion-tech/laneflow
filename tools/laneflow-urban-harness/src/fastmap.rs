//! harness 内部的整数键折叠哈希：键全部来自已验证输入，无 DoS 面，只为省 SipHash。

use std::hash::{BuildHasherDefault, Hasher};

pub(crate) type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[derive(Default)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn fold(&mut self, value: u64) {
        self.0 ^= value;
        self.0 = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 ^= self.0 >> 29;
    }
}

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0_u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.fold(u64::from_le_bytes(buf));
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.fold(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.fold(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.fold(value as u64);
    }
}
