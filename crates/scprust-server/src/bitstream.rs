//! Checked LSB-first primitives, not a netchannel or sign-on implementation.
//! Compound operations fail atomically. No unsafe/alignment-dependent loads.
pub const MAX_BYTES: usize = 65_536;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitError {
    Bounds,
    Width,
    Value,
    Unterminated,
    NonCanonical,
}
pub type Result<T> = std::result::Result<T, BitError>;

pub struct BitReader<'a> {
    bytes: &'a [u8],
    bits: usize,
    position: usize,
}
impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8], bits: usize) -> Result<Self> {
        if bytes.len() > MAX_BYTES || bits > bytes.len() * 8 {
            return Err(BitError::Bounds);
        }
        Ok(Self {
            bytes,
            bits,
            position: 0,
        })
    }
    pub fn position(&self) -> usize {
        self.position
    }
    pub fn remaining(&self) -> usize {
        self.bits - self.position
    }
    pub fn unsigned(&mut self, width: usize) -> Result<u32> {
        if width > 32 {
            return Err(BitError::Width);
        }
        if width > self.remaining() {
            return Err(BitError::Bounds);
        }
        let mut value = 0;
        for i in 0..width {
            let at = self.position + i;
            value |= u32::from((self.bytes[at / 8] >> (at % 8)) & 1) << i;
        }
        self.position += width;
        Ok(value)
    }
    pub fn signed(&mut self, width: usize) -> Result<i32> {
        if !(1..=32).contains(&width) {
            return Err(BitError::Width);
        }
        let value = self.unsigned(width)?;
        Ok(if width == 32 {
            value as i32
        } else {
            ((value << (32 - width)) as i32) >> (32 - width)
        })
    }
    /// Canonical unsigned LEB128, deliberately distinct from Source UBitVar.
    pub fn var_u32(&mut self) -> Result<u32> {
        let saved = self.position;
        let result = (|| {
            let mut value = 0u32;
            for i in 0..5 {
                let byte = self.unsigned(8)?;
                if i == 4 && byte > 0x0f {
                    return Err(BitError::Value);
                }
                value |= (byte & 0x7f) << (i * 7);
                if byte & 0x80 == 0 {
                    if i > 0 && byte == 0 {
                        return Err(BitError::NonCanonical);
                    }
                    return Ok(value);
                }
            }
            Err(BitError::Value)
        })();
        if result.is_err() {
            self.position = saved;
        }
        result
    }
    pub fn cstring(&mut self, max_bytes: usize) -> Result<Vec<u8>> {
        if max_bytes > MAX_BYTES {
            return Err(BitError::Bounds);
        }
        let saved = self.position;
        let result = (|| {
            let mut value = Vec::new();
            for _ in 0..=max_bytes {
                let byte = self.unsigned(8)? as u8;
                if byte == 0 {
                    return Ok(value);
                }
                if value.len() == max_bytes {
                    return Err(BitError::Unterminated);
                }
                value.push(byte);
            }
            Err(BitError::Unterminated)
        })();
        if result.is_err() {
            self.position = saved;
        }
        result
    }
}

pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
    capacity: usize,
}
impl BitWriter {
    pub fn new(capacity_bits: usize) -> Result<Self> {
        if capacity_bits > MAX_BYTES * 8 {
            return Err(BitError::Bounds);
        }
        Ok(Self {
            bytes: vec![0; capacity_bits.div_ceil(8)],
            bits: 0,
            capacity: capacity_bits,
        })
    }
    pub fn position(&self) -> usize {
        self.bits
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.bits.div_ceil(8)]
    }
    fn room(&self, bits: usize) -> Result<()> {
        if bits > self.capacity - self.bits {
            Err(BitError::Bounds)
        } else {
            Ok(())
        }
    }
    pub fn unsigned(&mut self, value: u32, width: usize) -> Result<()> {
        if width > 32 {
            return Err(BitError::Width);
        }
        if width < 32 && (value >> width) != 0 {
            return Err(BitError::Value);
        }
        self.room(width)?;
        for i in 0..width {
            let at = self.bits + i;
            self.bytes[at / 8] |= (((value >> i) & 1) as u8) << (at % 8);
        }
        self.bits += width;
        Ok(())
    }
    pub fn signed(&mut self, value: i32, width: usize) -> Result<()> {
        if !(1..=32).contains(&width) {
            return Err(BitError::Width);
        }
        if width < 32 {
            let limit = 1i64 << (width - 1);
            if i64::from(value) < -limit || i64::from(value) >= limit {
                return Err(BitError::Value);
            }
        }
        let mask = u32::MAX >> (32 - width);
        self.unsigned(value as u32 & mask, width)
    }
    pub fn var_u32(&mut self, mut value: u32) -> Result<()> {
        let mut encoded = [0; 5];
        let mut count = 0;
        loop {
            encoded[count] = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                encoded[count] |= 0x80;
            }
            count += 1;
            if value == 0 {
                break;
            }
        }
        self.room(count * 8)?;
        for byte in &encoded[..count] {
            self.unsigned(u32::from(*byte), 8)?;
        }
        Ok(())
    }
    pub fn cstring(&mut self, bytes: &[u8], max_bytes: usize) -> Result<()> {
        if bytes.len() > max_bytes || max_bytes > MAX_BYTES || bytes.contains(&0) {
            return Err(BitError::Value);
        }
        self.room((bytes.len() + 1) * 8)?;
        for byte in bytes {
            self.unsigned(u32::from(*byte), 8)?;
        }
        self.unsigned(0, 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn golden_lsb_and_signed_fields_cross_byte_boundaries() {
        let mut w = BitWriter::new(32).unwrap();
        w.unsigned(5, 3).unwrap();
        w.unsigned(0xab, 8).unwrap();
        w.signed(-3, 5).unwrap();
        assert_eq!(w.bytes(), &[0x5d, 0xed]);
        let mut r = BitReader::new(w.bytes(), w.position()).unwrap();
        assert_eq!(r.unsigned(3), Ok(5));
        assert_eq!(r.unsigned(8), Ok(0xab));
        assert_eq!(r.signed(5), Ok(-3));
        assert_eq!(r.remaining(), 0);
    }
    #[test]
    fn widths_offsets_and_boundary_values_roundtrip() {
        for offset in 0..8 {
            for width in 0..=32 {
                for value in [0, 1, u32::MAX, 0x12345678] {
                    let value = (u64::from(value) & ((1u64 << width) - 1)) as u32;
                    let mut w = BitWriter::new(80).unwrap();
                    w.unsigned(0, offset).unwrap();
                    w.unsigned(value, width).unwrap();
                    let mut r = BitReader::new(w.bytes(), w.position()).unwrap();
                    r.unsigned(offset).unwrap();
                    assert_eq!(r.unsigned(width), Ok(value));
                }
            }
        }
        for value in [i32::MIN, -1, 0, i32::MAX] {
            let mut w = BitWriter::new(32).unwrap();
            w.signed(value, 32).unwrap();
            assert_eq!(BitReader::new(w.bytes(), 32).unwrap().signed(32), Ok(value));
        }
    }
    #[test]
    fn canonical_varints_and_binary_strings() {
        let mut w = BitWriter::new(512).unwrap();
        w.unsigned(1, 1).unwrap();
        for v in [0, 127, 128, 16384, u32::MAX] {
            w.var_u32(v).unwrap();
        }
        w.cstring(b"\xff\x80test", 6).unwrap();
        let mut r = BitReader::new(w.bytes(), w.position()).unwrap();
        r.unsigned(1).unwrap();
        for v in [0, 127, 128, 16384, u32::MAX] {
            assert_eq!(r.var_u32(), Ok(v));
        }
        assert_eq!(r.cstring(6).unwrap(), b"\xff\x80test");
        let mut w = BitWriter::new(16).unwrap();
        w.var_u32(300).unwrap();
        assert_eq!(w.bytes(), &[0xac, 2]);
    }
    #[test]
    fn errors_do_not_consume_or_partially_write() {
        let mut w = BitWriter::new(8).unwrap();
        assert_eq!(w.var_u32(300), Err(BitError::Bounds));
        assert_eq!(w.signed(8, 4), Err(BitError::Value));
        assert_eq!(w.unsigned(1, 0), Err(BitError::Value));
        assert_eq!(w.unsigned(0, 33), Err(BitError::Width));
        assert_eq!(w.cstring(b"ab", 2), Err(BitError::Bounds));
        assert_eq!(w.position(), 0);
        for bytes in [&[0x80][..], &[0x80, 0], &[0xff, 0xff, 0xff, 0xff, 0x10]] {
            let mut r = BitReader::new(bytes, bytes.len() * 8).unwrap();
            assert!(r.var_u32().is_err());
            assert_eq!(r.position(), 0);
        }
        let mut r = BitReader::new(b"abc", 24).unwrap();
        assert!(r.cstring(2).is_err());
        assert_eq!(r.position(), 0);
        assert_eq!(r.unsigned(32), Err(BitError::Bounds));
        assert_eq!(r.position(), 0);
        assert!(BitWriter::new(MAX_BYTES * 8 + 1).is_err());
        assert!(BitReader::new(&[0], 9).is_err());
    }
}
