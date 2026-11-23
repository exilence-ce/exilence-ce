# Legacy import fixtures (F21)

Golden input for the Legacy import reader (`src-tauri/src/legacy_import/`) and the import golden test (`src-tauri/tests/core/legacy_import.rs`, which scans and imports both sets), per [parity verification](https://github.com/exilence-ce/migration/blob/main/docs/spec/parity-verification.md#legacy-import-f21). Written by the **real released legacy apps** (Electron 15.1.0 / Chromium 94); every value is synthetic and the GGG token is a fake.

| Directory | Written by | Holds |
|---|---|---|
| `both/exilence-ce-app/` | CE v1.3.0 (`Exilence-CE-1.2.11.AppImage`, the v1.3.0 release asset; its `package.json` says 1.2.11), sha256 `8c4c89d2…eac31` | `IndexedDB/`, `local-settings.json` |
| `both/exilence-next-app/` | CE v1.2.2 (`Exilence-Next-1.2.2.AppImage`), sha256 `80d250ac…50e20` | `IndexedDB/`, `local-settings.json` |
| `old-migration/exilence-next-app/` | CE v1.2.2 | `IndexedDB/`, `local-settings.json` |
| `expected/<set>/<folder>/<key>.json` | read in-page over CDP right before the app quit | the exact JSON string each live localForage key holds |

`both/` and `old-migration/` are each laid out like an `appData` directory, so a test can point the folder resolution at them.

## Cases

| Case | Where | Expected |
|---|---|---|
| Both folders hold the same Profile | Profile `e18a3763-0001-4000-8000-aaaaed000000` in `Foo` of both folders: 20 Snapshots in `exilence-next-app`, 2 newer ones in `exilence-ce-app` (CE 1.2.3 re-fetched Profiles) | `expected/both/*/account.json` |
| Legacy accounts `Foo` and `Foo#1234` | `exilence-ce-app` has both (`Foo#1234` with an empty-named Profile and one with 18 Snapshots); `exilence-next-app` has `Foo` | `expected/both/exilence-ce-app/account.json` |
| UTF-16 tab name | tab `Währung ✓ 通貨` of `Foo#1234`: `account` is a V8 two-byte string; `uiState` (`WTB ✓`) is an inline two-byte string with padding | `expected/both/exilence-ce-app/{account,uiState}.json` |
| Blob-wrapped value | `account` (> 64 KiB) in `IndexedDB/file__0.indexeddb.blob/1/00/<n>` in both folders: UTF-16 in CE, Latin-1 in Next | `account.json` |
| Overwrites, tombstones, `.ldb` table | in both `both/` folders `account` was written, then ~60 KB `pad*` values until LevelDB flushed its memtable to a `.ldb` table, the pads deleted, `account` overwritten, then re-written by the app on relaunch; `league` deleted in CE | no `pad*` key; `account.json` (20 Snapshots per folder) |
| `migration.current` below 3 | `old-migration/exilence-next-app`: `{"current":2}`, written after the app migrated so no launch wiped it | `expected/old-migration/exilence-next-app/migration.json` |
| Records without `currency`, `coffin`, `beast` | `exilence-next-app` `setting` has `showPriceInExalt: true` and no `currency` (Exilence Next ≤ 1.1.8 shape); its priced items have no `coffin`/`beast` (added in CE 1.2.10/1.2.11) | `expected/both/exilence-next-app/{setting,account}.json` |
| Custom prices of 0 or missing | `exilence-ce-app` `customPrice`, league `Settlers`: `Divine Orb` 150.5, `Mirror of Kalandra` 0, `Exalted Orb` missing, `Headhunter` 1500 | `expected/both/exilence-ce-app/customPrice.json` |
| `local-settings.json` | CE: `appExitAction` `exit`, set through the app's own `app-exit-action` IPC (its `setting` says `minimize-to-tray`); Next: legacy's defaults | the files themselves |
| GGG token | `account.token.accessToken` = `fake-token-never-imported` in both `both/` folders | the reader's output never contains it |

## Regenerating

Requires Node ≥ 22 (global `fetch`/`WebSocket`) and a display. Each app runs under `env -i` with `HOME` and `XDG_CONFIG_HOME` inside the scratch directory, so the user's real legacy data (`~/.config/exilence-ce-app`, …) is never read or modified.

```sh
mkdir -p /tmp/legacy-lab && cd /tmp/legacy-lab
gh release download v1.3.0 -R exilence-ce/exilence-ce-legacy -p '*.AppImage'
gh release download v1.2.2 -R exilence-ce/exilence-ce-legacy -p '*.AppImage'
chmod +x *.AppImage
mkdir ce next
(cd ce && ../Exilence-CE-1.2.11.AppImage --appimage-extract)
(cd next && ../Exilence-Next-1.2.2.AppImage --appimage-extract)
cd <repo>/fixtures/legacy-import
rm -rf both old-migration expected
DISPLAY=:0 node generate.mjs /tmp/legacy-lab/ce/squashfs-root /tmp/legacy-lab/next/squashfs-root /tmp/legacy-lab/work
```

[`generate.mjs`](generate.mjs) derives from the research lab script [`lab/cdp.mjs`](https://github.com/exilence-ce/migration/blob/research/indexeddb-import/research/indexeddb-import/lab/cdp.mjs). Data is deterministic, but LevelDB file layout (flush points, file numbers) can differ between runs, so regenerate `expected/` together with the folders. Generated on Linux (Arch, Wayland via XWayland) on 2026-11-11.
