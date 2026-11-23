//! Chromium 94's IndexedDB on top of LevelDB, and the Blink/V8 serialized value inside each
//! record. Legacy (localForage + mobx-persist with `jsonify`) only ever stores JSON strings under
//! string keys, so only those are decoded; anything else fails closed. Sources (Chromium
//! 94.0.4606.61): `content/browser/indexed_db/indexed_db_leveldb_coding.cc`,
//! `indexed_db_backing_store.cc`, `third_party/blink/renderer/modules/indexeddb/
//! idb_value_wrapping.cc`, `v8/src/objects/value-serializer.cc`.

use std::fs;
use std::path::Path;

use super::bytes::{Cursor, utf16};
use super::leveldb::Database;
use super::{FormatError, ReadError};

const DATABASE_NAME_TYPE_BYTE: u8 = 201;
const OBJECT_STORE_META_DATA_TYPE_BYTE: u8 = 50;
const OBJECT_STORE_NAME_META_DATA: u8 = 0;
const OBJECT_STORE_DATA_INDEX_ID: u64 = 1;
const BLOB_ENTRY_INDEX_ID: u64 = 3;
const IDB_KEY_STRING: u8 = 1;

/// The `(database, object store, index)` ids every IndexedDB LevelDB key starts with.
#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyPrefix {
    database: u64,
    store: u64,
    index: u64,
}

impl KeyPrefix {
    /// First byte: `(db len - 1) << 5 | (store len - 1) << 2 | (index len - 1)`, then the ids
    /// as minimal little-endian ints. Returns the prefix and the rest of the key.
    fn split(key: &[u8]) -> Option<(Self, &[u8])> {
        let first = *key.first()?;
        let lens = [
            usize::from(first >> 5 & 7) + 1,
            usize::from(first >> 2 & 7) + 1,
            usize::from(first & 3) + 1,
        ];
        let mut rest = &key[1..];
        let mut ids = [0; 3];
        for (id, len) in ids.iter_mut().zip(lens) {
            let (bytes, tail) = rest.split_at_checked(len)?;
            *id = le_int(bytes);
            rest = tail;
        }
        let [database, store, index] = ids;
        Some((
            Self {
                database,
                store,
                index,
            },
            rest,
        ))
    }

    fn encode(self) -> Vec<u8> {
        let len = |v: u64| (v.max(1).ilog2() / 8 + 1) as u8;
        let lens = [len(self.database), len(self.store), len(self.index)];
        let mut out = vec![(lens[0] - 1) << 5 | (lens[1] - 1) << 2 | (lens[2] - 1)];
        for (v, n) in [self.database, self.store, self.index]
            .into_iter()
            .zip(lens)
        {
            out.extend_from_slice(&v.to_le_bytes()[..usize::from(n)]);
        }
        out
    }
}

/// IndexedDB `EncodeInt`: minimal little-endian, at most 8 bytes.
fn le_int(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .take(8)
        .rev()
        .fold(0, |acc, &b| (acc << 8) | u64::from(b))
}

/// IndexedDB `StringWithLength`: varint(code units) + UTF-16BE.
fn string_with_length(c: &mut Cursor) -> Result<String, FormatError> {
    let units = c.varint_usize()?;
    let bytes = c.take(
        units
            .checked_mul(2)
            .ok_or_else(|| FormatError::new("string length"))?,
    )?;
    utf16(bytes, true)
}

/// One object store of one IndexedDB database, read from a copied `IndexedDB/` folder.
pub(super) struct ObjectStore<'a> {
    folder: &'a Path,
    db: &'a Database,
    database: u64,
    store: u64,
}

impl<'a> ObjectStore<'a> {
    /// Finds `store_name` in the database named `database_name`; `None` when either is absent.
    pub(super) fn find(
        folder: &'a Path,
        db: &'a Database,
        database_name: &str,
        store_name: &str,
    ) -> Result<Option<Self>, FormatError> {
        let Some(database) = find_database(db, database_name)? else {
            return Ok(None);
        };
        let Some(store) = find_store(db, database, store_name)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            folder,
            db,
            database,
            store,
        }))
    }

    /// Every live record with a string key.
    pub(super) fn records(&self) -> impl Iterator<Item = Result<Record<'a>, FormatError>> {
        let wanted = (self.database, self.store, OBJECT_STORE_DATA_INDEX_ID);
        self.db.live().filter_map(move |(key, value)| {
            let (prefix, encoded_key) = KeyPrefix::split(key)?;
            ((prefix.database, prefix.store, prefix.index) == wanted).then(|| {
                let mut c = Cursor::new(encoded_key);
                if c.byte()? != IDB_KEY_STRING {
                    return Err(FormatError::new("record key is not a string"));
                }
                let key = string_with_length(&mut c)?;
                Ok(Record {
                    key,
                    encoded_key,
                    value,
                })
            })
        })
    }

    /// The JS string a record holds. The record value is `varint(version) + SSV bytes`; SSVs over
    /// 64 KiB are wrapped by Blink into a blob file the record only references.
    pub(super) fn string_value(&self, record: &Record) -> Result<String, ReadError> {
        let mut c = Cursor::new(record.value);
        c.varint()?; // record version
        let ssv = c.rest();
        // `kVersionTag, 17 (pseudo version), kReplaceWithBlob, varint size, varint blob index`.
        if let Some(wrapped) = ssv.strip_prefix(&[0xff, 0x11, 0x01]) {
            let mut c = Cursor::new(wrapped);
            let size = c.varint()?;
            let slot = c.varint_usize()?;
            let bytes = fs::read(self.blob_path(record, slot)?)?;
            if bytes.len() as u64 != size {
                let message = format!("blob is {} bytes, record says {size}", bytes.len());
                return Err(FormatError::new(message).into());
            }
            Ok(ssv_string(&bytes)?)
        } else {
            Ok(ssv_string(ssv)?)
        }
    }

    /// `IndexedDB/file__0.indexeddb.blob/<db hex>/<(n >> 8) & 0xff as %02x>/<n hex>` for the
    /// blob in `slot` of the record's live blob entry (never by scanning the folder, which can
    /// hold stale blobs).
    fn blob_path(&self, record: &Record, slot: usize) -> Result<std::path::PathBuf, FormatError> {
        let prefix = KeyPrefix {
            database: self.database,
            store: self.store,
            index: BLOB_ENTRY_INDEX_ID,
        };
        let mut blob_key = prefix.encode();
        blob_key.extend_from_slice(record.encoded_key);
        let entry = self
            .db
            .get(&blob_key)
            .ok_or_else(|| FormatError::new(format!("blob entry of {:?} missing", record.key)))?;
        let number = *blob_numbers(entry)?
            .get(slot)
            .ok_or_else(|| FormatError::new("blob slot out of range"))?;
        Ok(self
            .folder
            .join("file__0.indexeddb.blob")
            .join(format!("{:x}", self.database))
            .join(format!("{:02x}", (number >> 8) & 0xff))
            .join(format!("{number:x}")))
    }
}

/// A live object store record under a string key.
pub(super) struct Record<'a> {
    pub(super) key: String,
    /// The IndexedDB-encoded key, shared by the record's blob entry.
    encoded_key: &'a [u8],
    value: &'a [u8],
}

/// Global key `prefix(0,0,0) + 201 + StringWithLength(origin) + StringWithLength(name)`; the
/// value is the database id.
fn find_database(db: &Database, name: &str) -> Result<Option<u64>, FormatError> {
    for (key, value) in db.live() {
        let Some((prefix, rest)) = KeyPrefix::split(key) else {
            continue;
        };
        let mut c = Cursor::new(rest);
        if (prefix.database, prefix.store, prefix.index) != (0, 0, 0)
            || c.peek() != Some(DATABASE_NAME_TYPE_BYTE)
        {
            continue;
        }
        c.byte()?;
        string_with_length(&mut c)?; // origin
        if string_with_length(&mut c)? == name {
            return Ok(Some(le_int(value)));
        }
    }
    Ok(None)
}

/// `prefix(db,0,0) + 50 + varint(store id) + 0 (name)`; the value is the UTF-16BE store name.
fn find_store(db: &Database, database: u64, name: &str) -> Result<Option<u64>, FormatError> {
    for (key, value) in db.live() {
        let Some((prefix, rest)) = KeyPrefix::split(key) else {
            continue;
        };
        let mut c = Cursor::new(rest);
        if (prefix.database, prefix.store, prefix.index) != (database, 0, 0)
            || c.peek() != Some(OBJECT_STORE_META_DATA_TYPE_BYTE)
        {
            continue;
        }
        c.byte()?;
        let id = c.varint()?;
        if c.peek() == Some(OBJECT_STORE_NAME_META_DATA) && utf16(value, true)? == name {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Blob numbers of a blob entry (`EncodeExternalObjects`): per object a type byte (0 blob,
/// 1 file, 2 File System Access handle), then its fields.
fn blob_numbers(entry: &[u8]) -> Result<Vec<u64>, FormatError> {
    let mut c = Cursor::new(entry);
    let mut out = Vec::new();
    while !c.is_empty() {
        match c.byte()? {
            kind @ (0 | 1) => {
                out.push(c.varint()?);
                string_with_length(&mut c)?; // MIME type
                c.varint()?; // size
                if kind == 1 {
                    string_with_length(&mut c)?; // file name
                    c.varint()?; // last modified
                }
            }
            2 => drop(c.len_prefixed()?),
            t => return Err(FormatError::new(format!("external object type {t}"))),
        }
    }
    Ok(out)
}

/// A Blink serialized value holding one V8 string: `FF <blink version>` (20 in Chromium 94;
/// 21 adds an `FE` trailer offset), `FF <v8 version>`, `00` padding, then `"` + Latin-1 or
/// `c` + UTF-16LE.
fn ssv_string(bytes: &[u8]) -> Result<String, FormatError> {
    let mut c = Cursor::new(bytes);
    if c.byte()? != 0xff {
        return Err(FormatError::new("serialized value lacks a version tag"));
    }
    let blink_version = c.varint()?;
    if blink_version > 21 {
        return Err(FormatError::new(format!(
            "Blink serialization version {blink_version}"
        )));
    }
    if blink_version == 21 && c.peek() == Some(0xfe) {
        c.take(1 + 8 + 4)?;
    }
    if c.peek() == Some(0xff) {
        c.byte()?;
        c.varint()?; // V8 serializer version
    }
    while c.peek() == Some(0) {
        c.byte()?; // kPadding aligns two-byte string payloads
    }
    match c.byte()? {
        b'"' => Ok(c.len_prefixed()?.iter().map(|&b| char::from(b)).collect()),
        b'c' => utf16(c.len_prefixed()?, false),
        tag => Err(FormatError::new(format!(
            "serialized value is not a string (tag {tag:#04x})"
        ))),
    }
}
