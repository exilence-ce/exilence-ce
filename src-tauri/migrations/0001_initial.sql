-- Exilence CE NG client: local SQLite schema, written as the first migration.
--
-- Decided in "What is the client's local SQLite schema?"
-- (https://github.com/exilence-ce/migration/issues/21). The Tauri core owns
-- all of it (docs/adr/0005-tauri-core-owns-client-state.md); the webview
-- stores nothing. The client owns Snapshot history
-- (docs/adr/0002-client-owns-snapshot-history.md). Wire contract:
-- docs/spec/exilence.proto. Backend counterpart: docs/spec/backend-schema.sql.
-- `F<n>`/`D<n>` refer to docs/spec/parity-checklist.md.
--
-- Storage
--
-- - Bundle identifier `com.exilence.exilence-ce-ng` (legacy:
--   `com.exilence.exilence-ce-app`, data folder `exilence-ce-app`). The
--   identifier names the app data directory, so it never changes.
-- - One file, `exilence-ce-ng.db`, in Tauri's app data directory. File mode
--   0600 on Unix; on Windows the per-user AppData ACL applies.
-- - sqlx 0.8 (SQLite). Connection options: foreign_keys ON, journal_mode WAL,
--   synchronous NORMAL, busy_timeout 5 s.
-- - Migrations are embedded (`sqlx::migrate!`), forward-only, and run at
--   startup before any window is built; this file is `0001_initial.sql`.
--   A database that has a migration this binary lacks (a newer version ran,
--   then an older one was installed by hand) blocks startup with a message:
--   install the newer version, or Reset data. Nothing is ever downgraded.
-- - Window size and position of the main window and the overlay (F14, F15)
--   live in `tauri-plugin-window-state`'s own file in the same directory, not
--   here.
--
-- Conventions
--
-- - Every table is STRICT.
-- - Account, Profile and Snapshot ids are canonical lowercase UUID text, as on
--   the wire. Profiles and Snapshots are keyed by (account_id, id), as on the
--   backend.
-- - Item definition ids are the wire's fixed64 xxh3-64 bits read as a signed
--   INTEGER.
-- - Times are INTEGER Unix milliseconds, UTC.
-- - Booleans are INTEGER 0/1.
-- - Values are chaos (REAL).
--
-- Per account vs per device
--
-- - Per GGG account: accounts, profiles, snapshots, snapshot_tabs,
--   pending_removals, legacy_import. Signing out deletes only the `login` row;
--   signing in as another account and back finds the first account's history
--   intact, as in legacy (`accountStore.accounts[]`).
-- - Per device: login, settings, custom_prices, rate_limit_policies,
--   item_definitions.
--
-- Not stored (memory only)
--
-- - Leagues, price leagues, stash tab lists, characters and poe.ninja prices:
--   fetched at start and when needed (legacy persisted the first four only as
--   caches).
-- - Income and its reset (F8).
-- - Notifications (F18), as in legacy. The F20 crash notice needs only the
--   `clean_exit` setting.
-- - Group state: members, their Profiles and Snapshots, player selection. A
--   reconnect inside a running app keeps them (D22); after an app restart the
--   client rejoins with empty cursors, like a first join (F13). Definitions
--   fetched for members do land in item_definitions.
--
-- Reset data (F17): sign out (best effort), close the pool, delete
-- `exilence-ce-ng.db` with its `-wal`/`-shm` files, the window-state file and
-- the webview data directory, then restart. Logs stay (F20). Legacy folders are
-- never touched.

-- ---------------------------------------------------------------- accounts

-- A GGG account that has signed in on this device.
CREATE TABLE accounts (
    -- GGG account uuid (Account.id).
    id                TEXT PRIMARY KEY,
    -- Display only; updated at every SignIn and StartSession.
    name              TEXT NOT NULL,
    -- StartSession's active_profile_id; NULL until the first Profile exists.
    -- Kept locally so an offline start still shows the account's history.
    active_profile_id TEXT,
    -- No ON DELETE action: the active Profile is switched before it is
    -- removed (F2), as on the backend.
    FOREIGN KEY (id, active_profile_id) REFERENCES profiles (account_id, id)
) STRICT;

-- ---------------------------------------------------------------- login

-- The signed-in Login (GLOSSARY: Login): at most one row. Its account is the
-- one whose data the app shows; no row means the login screen. Secrets live
-- here rather than in the OS keychain (Secret Service is often missing on
-- Linux tiling WMs); they never appear in a log or Diagnostics bundle (F20).
-- The GGG refresh token never reaches the client (ADR 0001).
CREATE TABLE login (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    account_id       TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Opaque backend bearer token (SignInResponse.session_token).
    session_token    TEXT NOT NULL,
    -- Replaced by every RefreshGggToken (D6).
    ggg_access_token TEXT NOT NULL,
    ggg_expires_at   INTEGER NOT NULL
) STRICT;

-- ---------------------------------------------------------------- profiles

-- A mirror of the backend's Profile list (GLOSSARY: Profile). Every Profile
-- change goes to the backend first and is written here after it succeeds
-- (Profile operations need the backend, F2).
CREATE TABLE profiles (
    account_id        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Client-minted UUIDv7, or the legacy uuid for an imported Profile.
    id                TEXT NOT NULL,
    -- Index in StartSession's list: creation order, as legacy appended them.
    position          INTEGER NOT NULL,
    name              TEXT NOT NULL CHECK (name <> ''),
    -- GGG league id.
    league_id         TEXT NOT NULL,
    -- poe.ninja league name (D21).
    price_league_id   TEXT NOT NULL,
    -- JSON array of GGG stash tab ids, in selection order.
    stash_tab_ids     TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(stash_tab_ids) AND json_type(stash_tab_ids) = 'array'),
    character_name    TEXT,
    include_equipment INTEGER NOT NULL DEFAULT 0 CHECK (include_equipment IN (0, 1)),
    include_inventory INTEGER NOT NULL DEFAULT 0 CHECK (include_inventory IN (0, 1)),
    PRIMARY KEY (account_id, id)
) STRICT;

-- ---------------------------------------------------------------- snapshots

-- The user's Snapshot history, the source of truth (ADR 0002): the backend
-- row's shape plus the bytes it was uploaded with; never rows per item.
CREATE TABLE snapshots (
    account_id  TEXT NOT NULL,
    -- Client-minted UUIDv7; an imported Snapshot's id comes from the import.
    id          TEXT NOT NULL,
    profile_id  TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    -- Sum of the Snapshot's snapshot_tabs values.
    total_value REAL NOT NULL,
    -- zstd-3 SnapshotBody, the exact bytes uploaded (or produced by the
    -- import). NULL once retention dropped the item detail (F12); never empty.
    body        BLOB CHECK (body IS NULL OR length(body) > 0),
    -- Brought in by the legacy import: never uploaded, so never in
    -- pending_removals (ADR 0002).
    imported    INTEGER NOT NULL DEFAULT 0 CHECK (imported IN (0, 1)),
    PRIMARY KEY (account_id, id),
    FOREIGN KEY (account_id, profile_id)
        REFERENCES profiles (account_id, id) ON DELETE CASCADE
) STRICT;

-- Newest first per Profile: latest Snapshot, sparkline (10), per-tab chart
-- (50), time spans, retention, Group cursors.
CREATE INDEX snapshots_newest
    ON snapshots (account_id, profile_id, created_at DESC, id DESC);

-- The wire's tab_values with each tab's name as of that Snapshot. Names are
-- kept here because bodies, which also carry them, survive only for the newest
-- 10 while the per-tab chart covers the newest 50 (F10).
CREATE TABLE snapshot_tabs (
    account_id  TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    -- Order of Snapshot.tab_values.
    position    INTEGER NOT NULL,
    -- GGG stash tab id; '' is the Character pseudo-tab (counted, not charted).
    tab_id      TEXT NOT NULL,
    name        TEXT NOT NULL,
    value       REAL NOT NULL,
    PRIMARY KEY (account_id, snapshot_id, position),
    FOREIGN KEY (account_id, snapshot_id)
        REFERENCES snapshots (account_id, id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Snapshot removals the backend has not confirmed yet (D5). Written in the
-- transaction that deletes the Snapshots locally; a row is deleted when
-- RemoveSnapshots succeeds for it, or when RemoveProfile succeeds for its
-- Profile. Sent only with that account's Login: right after the local delete,
-- then again after every successful StartSession, before any Group rejoin
-- (docs/spec/client-background-work.md). Uploads have no such table: solo, one
-- attempt, ignored on failure; in a Group, an in-memory retry queue (D27).
CREATE TABLE pending_removals (
    account_id  TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    snapshot_id TEXT NOT NULL,
    -- No foreign key: the Profile row may already be gone locally.
    profile_id  TEXT NOT NULL,
    PRIMARY KEY (account_id, snapshot_id)
) STRICT, WITHOUT ROWID;

-- ---------------------------------------------------------------- item definitions

-- Content-addressed and shared by every account on the device, like the
-- backend's table. Holds this device's definitions and those fetched for Group
-- members (GetItemDefinitions, SnapshotAdded).
CREATE TABLE item_definitions (
    -- xxh3-64 of `encoded`, checked before insert as on the backend.
    id       INTEGER PRIMARY KEY,
    -- prost encoding of the ItemDefinition with id 0.
    encoded  BLOB NOT NULL,
    -- The backend has it: set after an UploadSnapshot carrying it succeeds,
    -- and on insert for definitions that came from the backend. Unset
    -- definitions referenced by a new Snapshot go into new_definitions.
    uploaded INTEGER NOT NULL DEFAULT 0 CHECK (uploaded IN (0, 1))
) STRICT;

-- ---------------------------------------------------------------- custom prices

-- F6, per device as in legacy (`customPriceStore`). One row per poe.ninja
-- price entry of a price league, identified by legacy `findPrice`'s fields.
-- NULL means the price entry lacks the field (legacy `undefined`), which
-- matches only another absent field.
CREATE TABLE custom_prices (
    price_league_id TEXT NOT NULL,
    name            TEXT NOT NULL,
    icon            TEXT NOT NULL,
    frame_type      INTEGER,
    variant         TEXT,
    links           INTEGER,
    quality         INTEGER,
    level           INTEGER,
    ilvl            INTEGER,
    tier            INTEGER,
    corrupted       INTEGER CHECK (corrupted IN (0, 1)),
    elder           INTEGER CHECK (elder IN (0, 1)),
    shaper          INTEGER CHECK (shaper IN (0, 1)),
    -- Chaos.
    value           REAL NOT NULL CHECK (value >= 0)
) STRICT;

-- One custom price per entry; ifnull makes absent fields compare equal.
-- Numbers and booleans are never negative, so -1 cannot collide.
CREATE UNIQUE INDEX custom_prices_entry ON custom_prices (
    price_league_id, name, icon,
    ifnull(frame_type, -1), ifnull(variant, char(0)), ifnull(links, -1),
    ifnull(quality, -1), ifnull(level, -1), ifnull(ilvl, -1), ifnull(tier, -1),
    ifnull(corrupted, -1), ifnull(elder, -1), ifnull(shaper, -1)
);

-- ---------------------------------------------------------------- rate limits

-- F7: GGG's last rate-limit headers per policy, so a restart neither resets a
-- cooldown nor forgets recent usage. Per device. The limiter rebuilds from
-- these at startup (docs/spec/client-background-work.md): reported hits count
-- as sent at observed_at.
CREATE TABLE rate_limit_policies (
    -- X-Rate-Limit-Policy.
    policy           TEXT PRIMARY KEY,
    -- JSON object of every X-Rate-Limit-* header of the last response under
    -- this policy, name (lowercase) -> value, as received.
    headers          TEXT NOT NULL CHECK (json_valid(headers)),
    -- JSON array of the client's endpoint classes seen under this policy
    -- ("stash", "stash-list", "character", ...), so the first request after a
    -- restart is paced too.
    endpoints        TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(endpoints) AND json_type(endpoints) = 'array'),
    observed_at      INTEGER NOT NULL,
    -- End of the restriction after a 429: Retry-After, else the largest active
    -- restriction in the -State headers, else the largest restriction of the
    -- policy's rules. NULL otherwise.
    restricted_until INTEGER
) STRICT;

-- ---------------------------------------------------------------- settings

-- Every persisted setting and remembered UI choice, per device as in legacy.
-- Read into one typed struct in the app; a missing key means its default, so
-- a new setting needs no migration. Hardware acceleration and the release
-- channel are read before the first window is built.
--
-- Keys and defaults:
--   platform                  "pc" | "xbox" | "playstation"   "pc"     F1
--   auto_snapshot             bool                              false    F4
--   auto_snapshot_minutes     5..=1000                          20       F4
--   low_confidence_pricing    bool                              false    F5
--   price_threshold           0..=100                           0        F5
--   total_price_threshold     0..=5000                          0        F5
--   currency                  "chaos" | "exalted" | "divine"    "chaos"  F9
--   chart_time_span           "day" | "week" | "month" | "all"  "all"    F10
--   net_worth_chart_open      bool                              false    F10
--   tab_chart_open            bool                              false    F10
--   items_open                bool                              true     F10
--   item_table_sort           {column, descending}              legacy   F11
--   item_table_column_widths  {column: px}                      {}       F11
--   item_table_page_size      5 | 10 | 25 | 50                  25       F11
--   item_table_presets        [{name, hidden_columns}]          legacy   F11
--   item_table_active_preset  preset name                       legacy   F11
--   exit_action               "minimize_to_tray" | "exit"       "minimize_to_tray" F15
--   nav_drawer_open           bool                              false    F15
--   release_channel           "latest" | "beta"                 "latest" F16
--   ui_scale                  50..=150, step 10                 100      F17
--   hardware_acceleration     bool                              true     D7
--   tour_completed            bool                              false    F19
--   whats_new_shown           bool                              false    F19
--   debug_mode                bool                              false    F20
--   clean_exit                bool; false while running, true after a
--                             normal quit; false at launch means the
--                             last session crashed                true     F20
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL CHECK (json_valid(value))
) STRICT, WITHOUT ROWID;

-- ---------------------------------------------------------------- legacy import

-- Whether this account was offered the legacy import on this device and took
-- it or skipped it, so the offer never comes back for it (the Settings entry
-- still runs it). Mapping and flow: docs/spec/legacy-import.md, decided in
-- "How do legacy IndexedDB records map into the client's local schema?"
-- (https://github.com/exilence-ce/migration/issues/23).
CREATE TABLE legacy_import (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    state      TEXT NOT NULL CHECK (state IN ('done', 'declined')),
    at         INTEGER NOT NULL
) STRICT;

-- ---------------------------------------------------------------- operations
--
-- StartSession (F18): replace the account's Profiles with the response, in
-- one transaction (ADR 0002: the backend owns the list).
--
--   INSERT INTO accounts (id, name) VALUES (?1, ?2)
--   ON CONFLICT (id) DO UPDATE SET name = excluded.name;
--   -- per Profile, position = its index in the response:
--   INSERT INTO profiles (...) VALUES (...)
--   ON CONFLICT (account_id, id) DO UPDATE SET ... ;
--   UPDATE accounts SET active_profile_id = ?3 WHERE id = ?1;  -- NULL if ''
--   DELETE FROM pending_removals WHERE account_id = ?1
--     AND profile_id NOT IN (SELECT value FROM json_each(?4));
--   DELETE FROM profiles WHERE account_id = ?1
--     AND id NOT IN (SELECT value FROM json_each(?4));  -- cascades Snapshots
--
-- Retention (F12), in the transaction of every Snapshot insert and import,
-- per Profile (?1 account, ?2 Profile):
--
--   WITH ranked AS (
--     SELECT id, row_number() OVER (ORDER BY created_at DESC, id DESC) AS rn
--     FROM snapshots WHERE account_id = ?1 AND profile_id = ?2
--   )
--   UPDATE snapshots SET body = NULL
--   WHERE account_id = ?1 AND body IS NOT NULL
--     AND id IN (SELECT id FROM ranked WHERE rn > 10);
--
--   WITH ranked AS (...same...)
--   DELETE FROM snapshots
--   WHERE account_id = ?1 AND id IN (SELECT id FROM ranked WHERE rn > 1000);
--
-- Item definition sweep, once at startup before any Group rejoin: decode every
-- non-NULL body (at most 10 per Profile), collect the definition ids they
-- reference, and delete every other row of item_definitions. A swept
-- definition that a later Snapshot needs again is re-uploaded, which the
-- backend accepts.
--
-- Remove Snapshots (F12, D5), one transaction, then RemoveSnapshots:
--
--   INSERT INTO pending_removals (account_id, snapshot_id, profile_id)
--   SELECT account_id, id, profile_id FROM snapshots
--   WHERE account_id = ?1 AND id IN (...) AND imported = 0;
--   DELETE FROM snapshots WHERE account_id = ?1 AND id IN (...);
