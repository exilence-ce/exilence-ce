//! Read-only LevelDB scan: every live table and log of a database folder, resolved to the
//! newest entry per user key. Overwrites keep the highest sequence number and tombstones hide
//! older values, which is the answer a LevelDB `Get` gives. Every entry is read, so Chromium's
//! `idb_cmp1` key order is never needed. Formats: LevelDB `doc/log_format.md`,
//! `doc/table_format.md`, `db/version_edit.cc`.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use super::bytes::{Cursor, u32_le, u64_le};
use super::{FormatError, ReadError};

/// The comparator Chromium's IndexedDB backing store records in its MANIFEST.
const IDB_COMPARATOR: &str = "idb_cmp1";

struct Entry {
    sequence: u64,
    /// `None` for a tombstone.
    value: Option<Vec<u8>>,
}

/// The newest entry per user key across a database's live files.
#[derive(Default)]
pub(super) struct Database(HashMap<Vec<u8>, Entry>);

impl Database {
    /// Reads the database in `dir` (a `*.leveldb` folder): live `.ldb` tables, then live logs.
    pub(super) fn read(dir: &Path) -> Result<Self, ReadError> {
        let files = LiveFiles::read(dir)?;
        if files.comparator != IDB_COMPARATOR {
            return Err(FormatError::new(format!("comparator {:?}", files.comparator)).into());
        }
        let mut db = Self::default();
        for number in files.tables {
            let ldb = dir.join(format!("{number:06}.ldb"));
            let path = if ldb.exists() {
                ldb
            } else {
                dir.join(format!("{number:06}.sst"))
            };
            db.apply_table(&fs::read(path)?)?;
        }
        for path in files.logs {
            for batch in log_records(&fs::read(path)?) {
                db.apply_write_batch(&batch)?;
            }
        }
        Ok(db)
    }

    /// The live value of `key`.
    pub(super) fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.0.get(key)?.value.as_deref()
    }

    /// Every live key and value, in no particular order.
    pub(super) fn live(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.0
            .iter()
            .filter_map(|(k, e)| Some((k.as_slice(), e.value.as_deref()?)))
    }

    /// Keys whose newest entry is a deletion.
    #[cfg(test)]
    pub(super) fn tombstones(&self) -> usize {
        self.0.values().filter(|e| e.value.is_none()).count()
    }

    fn offer(&mut self, key: &[u8], sequence: u64, value: Option<&[u8]>) {
        if self.0.get(key).is_some_and(|e| e.sequence >= sequence) {
            return;
        }
        let value = value.map(<[u8]>::to_vec);
        self.0.insert(key.to_vec(), Entry { sequence, value });
    }

    /// A log record: `sequence u64 | count u32 | (tag, key[, value])*`.
    fn apply_write_batch(&mut self, batch: &[u8]) -> Result<(), FormatError> {
        let mut c = Cursor::new(batch);
        let sequence = u64_le(c.take(8)?);
        let count = u32_le(c.take(4)?);
        for i in 0..u64::from(count) {
            let tag = c.byte()?;
            let key = c.len_prefixed()?;
            match tag {
                1 => self.offer(key, sequence + i, Some(c.len_prefixed()?)),
                0 => self.offer(key, sequence + i, None),
                t => return Err(FormatError::new(format!("write batch tag {t}"))),
            }
        }
        Ok(())
    }

    /// A sorted table: footer → index block → data blocks of internal keys
    /// (`user key | sequence << 8 | kind`, kind 1 = value, 0 = deletion).
    fn apply_table(&mut self, file: &[u8]) -> Result<(), FormatError> {
        const MAGIC: u64 = 0xdb47_7524_8b80_fb57;
        let footer = file
            .len()
            .checked_sub(48)
            .map(|start| &file[start..])
            .ok_or_else(|| FormatError::new("table shorter than its footer"))?;
        if u64_le(&footer[40..]) != MAGIC {
            return Err(FormatError::new("table magic"));
        }
        let mut c = Cursor::new(footer);
        BlockHandle::read(&mut c)?; // metaindex
        let index = BlockHandle::read(&mut c)?;
        for (_, handle) in block_entries(&index.block(file)?)? {
            let data = BlockHandle::read(&mut Cursor::new(handle))?.block(file)?;
            for (internal_key, value) in block_entries(&data)? {
                let split = internal_key
                    .len()
                    .checked_sub(8)
                    .ok_or_else(|| FormatError::new("internal key"))?;
                let trailer = u64_le(&internal_key[split..]);
                let value = (trailer & 0xff == 1).then_some(value);
                self.offer(&internal_key[..split], trailer >> 8, value);
            }
        }
        Ok(())
    }
}

struct BlockHandle {
    offset: usize,
    size: usize,
}

impl BlockHandle {
    fn read(c: &mut Cursor) -> Result<Self, FormatError> {
        Ok(Self {
            offset: c.varint_usize()?,
            size: c.varint_usize()?,
        })
    }

    /// The block's contents; a trailer byte after it says raw (0) or snappy (1).
    fn block(&self, file: &[u8]) -> Result<Vec<u8>, FormatError> {
        let end = self.offset.saturating_add(self.size);
        let raw = file
            .get(self.offset..end)
            .ok_or_else(|| FormatError::new("block out of range"))?;
        match file.get(end) {
            Some(0) => Ok(raw.to_vec()),
            Some(1) => snap::raw::Decoder::new()
                .decompress_vec(raw)
                .map_err(|e| FormatError::new(format!("snappy: {e}"))),
            other => Err(FormatError::new(format!("block compression {other:?}"))),
        }
    }
}

/// A table block entry: full key, value.
type BlockEntry<'a> = (Vec<u8>, &'a [u8]);

/// Entries of a table block: prefix-compressed keys, then restart points.
fn block_entries(block: &[u8]) -> Result<Vec<BlockEntry<'_>>, FormatError> {
    let short = || FormatError::new("block restarts");
    let restarts_at = block.len().checked_sub(4).ok_or_else(short)?;
    let restarts = u32_le(&block[restarts_at..]) as usize;
    let limit = restarts
        .checked_mul(4)
        .and_then(|n| restarts_at.checked_sub(n))
        .ok_or_else(short)?;
    let mut c = Cursor::new(&block[..limit]);
    let (mut out, mut key) = (Vec::new(), Vec::new());
    while !c.is_empty() {
        let shared = c.varint_usize()?;
        let unshared = c.varint_usize()?;
        let value_len = c.varint_usize()?;
        if shared > key.len() {
            return Err(FormatError::new("block key prefix"));
        }
        key.truncate(shared);
        key.extend_from_slice(c.take(unshared)?);
        out.push((key.clone(), c.take(value_len)?));
    }
    Ok(out)
}

/// Records of a log file (`*.log`, `MANIFEST-*`): 32 KiB blocks of CRC32C-checked fragments.
/// Reading stops at the first torn or corrupt fragment: what a crash left half-written was never
/// acknowledged to the writer.
fn log_records(data: &[u8]) -> Vec<Vec<u8>> {
    const BLOCK: usize = 32 * 1024;
    const HEADER: usize = 7;
    let (mut out, mut partial, mut pos) = (Vec::new(), Vec::new(), 0);
    while pos + HEADER <= data.len() {
        let left = BLOCK - pos % BLOCK;
        if left < HEADER {
            pos += left; // block trailer
            continue;
        }
        let header = &data[pos..pos + HEADER];
        let len = usize::from(u16::from_le_bytes([header[4], header[5]]));
        let kind = header[6];
        let Some(fragment) = data.get(pos + HEADER..pos + HEADER + len) else {
            break;
        };
        if kind == 0 && len == 0 {
            pos += left; // preallocated zeroes up to the block end
            continue;
        }
        let mut crc = crc32c::crc32c(&[kind]);
        crc = crc32c::crc32c_append(crc, fragment);
        if unmask_crc(u32_le(header)) != crc {
            break;
        }
        pos += HEADER + len;
        match kind {
            1 => out.push(fragment.to_vec()),
            2 => partial = fragment.to_vec(),
            3 => partial.extend_from_slice(fragment),
            4 => {
                partial.extend_from_slice(fragment);
                out.push(std::mem::take(&mut partial));
            }
            _ => break,
        }
    }
    out
}

/// LevelDB stores CRCs masked (`util/crc32c.h`).
fn unmask_crc(masked: u32) -> u32 {
    let rot = masked.wrapping_sub(0xa282_ead8);
    rot.rotate_left(15)
}

/// The live file set named by `CURRENT` → `MANIFEST-*` version edits.
struct LiveFiles {
    comparator: String,
    tables: BTreeSet<u64>,
    /// Logs not yet flushed to a table, oldest first.
    logs: Vec<std::path::PathBuf>,
}

impl LiveFiles {
    fn read(dir: &Path) -> Result<Self, ReadError> {
        let current = fs::read_to_string(dir.join("CURRENT"))?;
        let manifest = fs::read(dir.join(current.trim()))?;
        let (mut comparator, mut log_number, mut prev_log) = (String::new(), 0, 0);
        let mut tables = BTreeSet::new();
        for edit in log_records(&manifest) {
            let mut c = Cursor::new(&edit);
            while !c.is_empty() {
                match c.varint()? {
                    1 => comparator = String::from_utf8_lossy(c.len_prefixed()?).into_owned(),
                    2 => log_number = c.varint()?,
                    3 | 4 => drop(c.varint()?), // next file number, last sequence
                    5 => {
                        c.varint()?; // compact pointer: level, internal key
                        c.len_prefixed()?;
                    }
                    6 => {
                        c.varint()?; // deleted file: level, number
                        tables.remove(&c.varint()?);
                    }
                    7 => {
                        c.varint()?; // new file: level, number, size, smallest, largest
                        tables.insert(c.varint()?);
                        c.varint()?;
                        c.len_prefixed()?;
                        c.len_prefixed()?;
                    }
                    9 => prev_log = c.varint()?,
                    t => return Err(FormatError::new(format!("manifest tag {t}")).into()),
                }
            }
        }
        let mut logs = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let number = path
                .file_name()
                .and_then(|n| n.to_str()?.strip_suffix(".log")?.parse::<u64>().ok());
            if let Some(n) = number.filter(|&n| n >= log_number || n == prev_log) {
                logs.push((n, path));
            }
        }
        logs.sort();
        Ok(Self {
            comparator,
            tables,
            logs: logs.into_iter().map(|(_, p)| p).collect(),
        })
    }
}
