//! Reads one legacy data folder (Electron `userData` of Exilence CE or Exilence Next) without
//! Chromium: LevelDB → IndexedDB records → Blink/V8 serialized string → JSON → the lenient
//! [`model`](super::model) types. `IndexedDB/` is copied to a temp directory first and read
//! there; the legacy folder is never written. Format notes:
//! <https://github.com/exilence-ce/migration/blob/research/indexeddb-import/research/indexeddb-import/findings.md>.

mod bytes;
mod idb;
mod leveldb;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;

use super::model::{AccountStore, CustomPriceStore, LocalSettings, Migration, Setting, UiState};
use idb::ObjectStore;
use leveldb::Database;

/// localForage's database (`localForage.config({ name })` in legacy `src/index.tsx`).
const DATABASE: &str = "exilence-next-db";
/// localForage's default object store.
const OBJECT_STORE: &str = "keyvaluepairs";
const INDEXED_DB: &str = "IndexedDB";
const LEVELDB: &str = "file__0.indexeddb.leveldb";
const LOCAL_SETTINGS: &str = "local-settings.json";

/// The mobx-persist keys the import reads. `league`, `rateLimit` and anything else is skipped
/// undecoded.
const KEYS: [&str; 5] = ["account", "customPrice", "setting", "uiState", "migration"];

/// Everything the import reads from one legacy folder. A value legacy never wrote is `None`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LegacyData {
    pub migration: Option<Migration>,
    /// Without the GGG token, which is never deserialized.
    pub account: Option<AccountStore>,
    pub custom_price: Option<CustomPriceStore>,
    pub setting: Option<Setting>,
    pub ui_state: Option<UiState>,
    /// `local-settings.json` from the folder root.
    pub local_settings: Option<LocalSettings>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// Legacy holds its LevelDB `LOCK`: Exilence CE is running and must be quit first.
    #[error("Exilence CE is running")]
    Locked,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("unreadable legacy data: {0}")]
    Format(#[from] FormatError),
    #[error("legacy {key} is not the expected JSON: {source}")]
    Json {
        key: &'static str,
        source: serde_json::Error,
    },
}

/// Data that is not what Chromium 94 / legacy writes.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct FormatError(String);

impl FormatError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Reads the legacy folder `folder` (e.g. `~/.config/exilence-ce-app`). A folder without
/// IndexedDB or `local-settings.json` yields `None` values. Fails with [`ReadError::Locked`]
/// while legacy runs, checked before and after the copy so a copy torn by a starting legacy is
/// never decoded.
pub fn read_folder(folder: &Path) -> Result<LegacyData, ReadError> {
    let mut values = BTreeMap::new();
    let indexed_db = folder.join(INDEXED_DB);
    if indexed_db.join(LEVELDB).is_dir() {
        let lock = indexed_db.join(LEVELDB).join("LOCK");
        ensure_unlocked(&lock)?;
        let temp = tempfile::tempdir()?;
        let copy = temp.path().join(INDEXED_DB);
        copy_dir(&indexed_db, &copy)?;
        ensure_unlocked(&lock)?;
        values = read_strings(&copy, |key| KEYS.contains(&key))?;
    }
    let local_settings = match fs::read_to_string(folder.join(LOCAL_SETTINGS)) {
        Ok(json) => Some(parse(LOCAL_SETTINGS, &json)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let mut take = |key: &'static str| values.remove(key).map(|json| (key, json));
    Ok(LegacyData {
        migration: take("migration").map(|(k, v)| parse(k, &v)).transpose()?,
        account: take("account").map(|(k, v)| parse(k, &v)).transpose()?,
        custom_price: take("customPrice").map(|(k, v)| parse(k, &v)).transpose()?,
        setting: take("setting").map(|(k, v)| parse(k, &v)).transpose()?,
        ui_state: take("uiState").map(|(k, v)| parse(k, &v)).transpose()?,
        local_settings,
    })
}

fn parse<T: DeserializeOwned>(key: &'static str, json: &str) -> Result<T, ReadError> {
    serde_json::from_str(json).map_err(|source| ReadError::Json { key, source })
}

/// The JSON string of every live localForage key in the `IndexedDB/` folder `indexed_db` that
/// `keep` accepts. Rejected keys are never decoded past their name.
fn read_strings(
    indexed_db: &Path,
    keep: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, String>, ReadError> {
    let db = Database::read(&indexed_db.join(LEVELDB))?;
    let Some(store) = ObjectStore::find(indexed_db, &db, DATABASE, OBJECT_STORE)? else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for record in store.records() {
        let record = record?;
        if keep(&record.key) {
            let value = store.string_value(&record)?;
            out.insert(record.key, value);
        }
    }
    Ok(out)
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn ensure_unlocked(lock: &Path) -> Result<(), ReadError> {
    let file = match File::open(lock) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if is_locked(&file)? {
        Err(ReadError::Locked)
    } else {
        Ok(())
    }
}

/// Chromium's LevelDB env locks `LOCK` with `fcntl(F_SETLK, F_WRLCK)` (POSIX record locks,
/// which `flock` does not see), so ask with `F_GETLK`.
#[cfg(unix)]
fn is_locked(file: &File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    // SAFETY: `flock` is plain old data; all-zero is a valid value.
    let mut query: libc::flock = unsafe { std::mem::zeroed() };
    query.l_type = libc::F_WRLCK as _;
    query.l_whence = libc::SEEK_SET as _;
    // SAFETY: a valid open descriptor and a valid `flock` the call writes into.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut query) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // `l_type` is a `c_short`; `F_UNLCK` is a `c_int` on Linux, a `c_short` on macOS.
    Ok(query.l_type != libc::F_UNLCK as libc::c_short)
}

/// Chromium locks `LOCK` with an exclusive `LockFileEx`, which a shared lock attempt sees.
#[cfg(windows)]
fn is_locked(file: &File) -> io::Result<bool> {
    match file.try_lock_shared() {
        Ok(()) => Ok(false), // released when `file` drops
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests;
