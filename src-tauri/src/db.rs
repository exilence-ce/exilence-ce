//! The local SQLite database (`client-schema.sql`): one file in the app data
//! directory, migrated at startup before any window is built.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};

/// The database file in the app data directory.
pub const FILE_NAME: &str = "exilence-ce-ng.db";

/// Embedded, forward-only migrations; `0001_initial.sql` is the client schema.
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Why the database could not be opened.
#[derive(Debug)]
pub enum OpenError {
    Io(std::io::Error),
    Sqlx(sqlx::Error),
    /// A newer version of the app migrated the database; nothing is ever
    /// downgraded.
    NewerVersion {
        migration: i64,
    },
    Migrate(MigrateError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot create the database file: {e}"),
            Self::Sqlx(e) => write!(f, "cannot open the database: {e}"),
            Self::NewerVersion { migration } => write!(
                f,
                "the database was written by a newer version of Exilence CE NG \
                 (migration {migration}); install the newer version, or Reset data"
            ),
            Self::Migrate(e) => write!(f, "cannot migrate the database: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Opens (creating if missing) the database in `dir` and runs the migrations.
pub async fn open(dir: &Path) -> Result<SqlitePool, OpenError> {
    std::fs::create_dir_all(dir).map_err(OpenError::Io)?;
    let path = dir.join(FILE_NAME);
    create_private(&path).map_err(OpenError::Io)?;
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .pragma("temp_store", "MEMORY")
        .pragma("cache_size", "-16000")
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePool::connect_with(options)
        .await
        .map_err(OpenError::Sqlx)?;
    MIGRATOR.run(&pool).await.map_err(|e| match e {
        MigrateError::VersionMissing(migration) => OpenError::NewerVersion { migration },
        e => OpenError::Migrate(e),
    })?;
    Ok(pool)
}

/// `value` as the JSON text SQLite's `json_each` reads. Only a map with
/// non-string keys can fail; the error is the bind's.
pub fn json_text<T: serde::Serialize + ?Sized>(value: &T) -> sqlx::Result<String> {
    serde_json::to_string(value).map_err(|e| sqlx::Error::Encode(Box::new(e)))
}

/// Creates `path` if missing, readable only by the user on Unix. SQLite gives
/// its `-wal` and `-shm` files the database file's mode.
fn create_private(path: &Path) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path).map(drop)
}
