//! Bounds-checked byte decoding shared by the LevelDB and IndexedDB layers.

use super::FormatError;

pub(super) struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub(super) fn rest(&self) -> &'a [u8] {
        &self.bytes[self.pos.min(self.bytes.len())..]
    }

    pub(super) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let end = self.pos.checked_add(n);
        let slice = end
            .and_then(|end| self.bytes.get(self.pos..end))
            .ok_or_else(|| FormatError::new("unexpected end of data"))?;
        self.pos += n;
        Ok(slice)
    }

    pub(super) fn byte(&mut self) -> Result<u8, FormatError> {
        Ok(self.take(1)?[0])
    }

    /// LevelDB/V8 base-128 varint.
    pub(super) fn varint(&mut self) -> Result<u64, FormatError> {
        let mut out = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(FormatError::new("varint overflow"))
    }

    pub(super) fn varint_usize(&mut self) -> Result<usize, FormatError> {
        usize::try_from(self.varint()?).map_err(|_| FormatError::new("length overflow"))
    }

    /// `varint(len) + len bytes`.
    pub(super) fn len_prefixed(&mut self) -> Result<&'a [u8], FormatError> {
        let n = self.varint_usize()?;
        self.take(n)
    }
}

pub(super) fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub(super) fn u64_le(b: &[u8]) -> u64 {
    let mut out = [0; 8];
    out.copy_from_slice(&b[..8]);
    u64::from_le_bytes(out)
}

/// UTF-16 code units in big-endian (IndexedDB key coding) or little-endian (V8) byte order.
pub(super) fn utf16(bytes: &[u8], big_endian: bool) -> Result<String, FormatError> {
    let (pairs, []) = bytes.as_chunks::<2>() else {
        return Err(FormatError::new("odd UTF-16 byte length"));
    };
    let units = pairs.iter().map(|&pair| {
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|e| FormatError::new(format!("UTF-16: {e}")))
}
