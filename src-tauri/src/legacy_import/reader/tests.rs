//! Golden tests over the committed fixtures (`fixtures/legacy-import/`), written by the real
//! legacy AppImages; `expected/` holds the exact JSON string each key held, read in-page.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/legacy-import")
}

/// Every (set, legacy folder) the generator wrote.
const FOLDERS: [(&str, &str); 3] = [
    ("both", "exilence-ce-app"),
    ("both", "exilence-next-app"),
    ("old-migration", "exilence-next-app"),
];

fn expected(set: &str, app: &str) -> BTreeMap<String, String> {
    let dir = fixtures().join("expected").join(set).join(app);
    fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            let key = path.file_stem().unwrap().to_str().unwrap().to_owned();
            (key, fs::read_to_string(path).unwrap())
        })
        .collect()
}

fn expected_value<T: DeserializeOwned>(set: &str, app: &str, key: &str) -> Option<T> {
    expected(set, app)
        .get(key)
        .map(|json| serde_json::from_str(json).unwrap())
}

#[test]
fn f21_every_fixture_decodes_to_the_json_legacy_wrote() {
    for (set, app) in FOLDERS {
        let indexed_db = fixtures().join(set).join(app).join(INDEXED_DB);
        // Every live key, not only the imported ones: deleted `pad*` keys must not resurface.
        let decoded = read_strings(&indexed_db, |_| true).unwrap();
        assert_eq!(decoded, expected(set, app), "{set}/{app}");
    }
}

#[test]
fn f21_overwrites_and_tombstones_resolve_to_the_live_value() {
    for app in ["exilence-ce-app", "exilence-next-app"] {
        let folder = fixtures().join("both").join(app).join(INDEXED_DB);
        let leveldb = folder.join(LEVELDB);
        let files: Vec<String> = fs::read_dir(&leveldb)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(
            files.iter().any(|f| f.ends_with(".ldb")),
            "{app}: no .ldb table"
        );
        let db = Database::read(&leveldb).unwrap();
        assert!(db.tombstones() > 0, "{app}: no tombstones");
        // `account` was first written before the memtable flush (so its first value sits in a
        // `.ldb` table), overwritten, then re-written by the app on its next launch.
        let account: AccountStore = expected_value("both", app, "account").unwrap();
        let read = read_folder(&fixtures().join("both").join(app)).unwrap();
        assert_eq!(read.account, Some(account));
        let snapshots: usize = read
            .account
            .unwrap()
            .accounts
            .unwrap()
            .iter()
            .flat_map(|a| a.profiles.iter().flatten())
            .map(|p| p.snapshots.as_ref().map_or(0, Vec::len))
            .sum();
        assert_eq!(snapshots, 20, "{app}");
    }
}

#[test]
fn f21_read_folder_yields_the_imported_keys_typed() {
    for (set, app) in FOLDERS {
        let read = read_folder(&fixtures().join(set).join(app)).unwrap();
        let local_settings =
            fs::read_to_string(fixtures().join(set).join(app).join(LOCAL_SETTINGS)).unwrap();
        let want = LegacyData {
            migration: expected_value(set, app, "migration"),
            account: expected_value(set, app, "account"),
            custom_price: expected_value(set, app, "customPrice"),
            setting: expected_value(set, app, "setting"),
            ui_state: expected_value(set, app, "uiState"),
            local_settings: Some(serde_json::from_str(&local_settings).unwrap()),
        };
        assert_eq!(read, want, "{set}/{app}");
    }
}

#[test]
fn f21_fixture_cases() {
    let ce = read_folder(&fixtures().join("both/exilence-ce-app")).unwrap();
    let next = read_folder(&fixtures().join("both/exilence-next-app")).unwrap();
    let old = read_folder(&fixtures().join("old-migration/exilence-next-app")).unwrap();

    // Only `migration.current` 3 is importable; the old folder is below.
    assert_eq!(ce.migration.as_ref().unwrap().current, Some(3));
    assert_eq!(next.migration.as_ref().unwrap().current, Some(3));
    assert_eq!(old.migration.unwrap().current, Some(2));

    // Accounts `Foo` and `Foo#1234`; both folders hold the same Profile.
    let accounts = |d: &LegacyData| d.account.clone().unwrap().accounts.unwrap();
    let names = |d: &LegacyData| {
        accounts(d)
            .into_iter()
            .map(|a| a.name.unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&ce), ["Foo", "Foo#1234"]);
    assert_eq!(names(&next), ["Foo"]);
    let profile_ids = |d: &LegacyData| {
        accounts(d)
            .into_iter()
            .flat_map(|a| a.profiles.unwrap())
            .map(|p| p.uuid.unwrap())
            .collect::<Vec<_>>()
    };
    let shared = &profile_ids(&next)[0];
    assert!(profile_ids(&ce).contains(shared));

    // A UTF-16 tab name, in the account's tab list and in the newest priced items.
    let foo_1234 = &accounts(&ce)[1];
    let tabs = foo_1234.account_leagues.as_ref().unwrap()[0]
        .stashtabs
        .as_ref()
        .unwrap();
    assert_eq!(tabs[0].name.as_deref(), Some("Währung ✓ 通貨"));
    let newest = &foo_1234.profiles.as_ref().unwrap()[1]
        .snapshots
        .as_ref()
        .unwrap()[0];
    let item_tab = &newest.stash_tab_snapshots.as_ref().unwrap()[0]
        .priced_items
        .as_ref()
        .unwrap()[0];
    assert_eq!(item_tab.tab.as_ref().unwrap()[0].name, tabs[0].name);

    // Blob-wrapped values: `account` lives in a blob file in both folders.
    for app in ["exilence-ce-app", "exilence-next-app"] {
        let blobs = fixtures()
            .join("both")
            .join(app)
            .join("IndexedDB/file__0.indexeddb.blob/1/00");
        assert_eq!(fs::read_dir(blobs).unwrap().count(), 1, "{app}");
    }

    // Records without `currency` (setting) or `coffin`/`beast` (CE ≤ 1.2.2 priced items).
    let setting = next.setting.unwrap();
    assert_eq!(
        (setting.currency, setting.show_price_in_exalt),
        (None, Some(true))
    );
    assert_eq!(ce.setting.unwrap().currency.as_deref(), Some("exalt"));
    let next_account = &expected("both", "exilence-next-app")["account"];
    assert!(
        next_account.contains("pricedItems\":[{")
            && !next_account.contains("coffin")
            && !next_account.contains("beast")
    );
    assert!(
        expected("both", "exilence-ce-app")["account"].contains("\"coffin\":false,\"beast\":false")
    );

    // Custom prices of 0 and missing are read as such.
    let prices = ce.custom_price.unwrap().custom_league_prices.unwrap()[0]
        .prices
        .clone()
        .unwrap();
    let custom: Vec<_> = prices.iter().map(|p| p.custom_price).collect();
    assert_eq!(custom, [Some(150.5), Some(0.0), None, Some(1500.0)]);

    // `local-settings.json` is read alongside (the app wrote `exit` through its IPC).
    assert_eq!(
        ce.local_settings.unwrap().app_exit_action.as_deref(),
        Some("exit")
    );
}

#[test]
fn f21_token_is_never_kept() {
    let folder = fixtures().join("both/exilence-ce-app");
    assert!(expected("both", "exilence-ce-app")["account"].contains("fake-token-never-imported"));
    let read = read_folder(&folder).unwrap();
    assert!(!format!("{read:?}").contains("fake-token-never-imported"));
}

#[test]
fn f21_missing_folder_reads_nothing() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(read_folder(temp.path()).unwrap(), LegacyData::default());
}

/// Legacy holds `LOCK` with a POSIX record lock; an open file description lock from this process
/// conflicts with it the same way (and `F_GETLK` reports it), so it stands in for a running legacy.
#[cfg(target_os = "linux")]
#[test]
fn f21_lock_held_by_legacy_refuses() {
    use std::os::fd::AsRawFd;

    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("exilence-ce-app");
    copy_dir(&fixtures().join("both/exilence-ce-app"), &folder).unwrap();
    let lock = File::options()
        .read(true)
        .write(true)
        .open(folder.join(INDEXED_DB).join(LEVELDB).join("LOCK"))
        .unwrap();
    // SAFETY: plain old data, then a valid descriptor and `flock`.
    let mut request: libc::flock = unsafe { std::mem::zeroed() };
    request.l_type = libc::F_WRLCK as _;
    request.l_whence = libc::SEEK_SET as _;
    assert_eq!(
        unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_OFD_SETLK, &request) },
        0
    );

    assert!(matches!(read_folder(&folder), Err(ReadError::Locked)));

    drop(lock);
    assert!(read_folder(&folder).unwrap().account.is_some());
}
