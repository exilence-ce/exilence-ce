//! Step 4 of the import: one local transaction with Profiles, Snapshots, `snapshot_tabs`, custom
//! prices, settings, F12 retention per Profile and the `legacy_import` row
//! ([merge rules](https://github.com/exilence-ce/migration/blob/main/docs/spec/legacy-import.md#merge-rules)).

use ipc::slices::Profile;
use sqlx::SqlitePool;

use super::mapping::{Scan, Selection};
use crate::{db, settings, snapshot};

/// What the transaction writes.
pub struct Write<'a> {
    pub account_id: &'a str,
    pub scan: &'a Scan,
    pub selection: &'a Selection,
    /// The Profiles the account lacked, with their positions, already created on the backend.
    pub new_profiles: &'a [(i64, Profile)],
    /// Set when the account had no Profile.
    pub active_profile_id: Option<&'a str>,
    /// Unix ms.
    pub at: i64,
}

/// What the import added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Imported {
    /// Profiles created.
    pub profiles: u32,
    /// Snapshots added and kept by retention.
    pub snapshots: u32,
}

/// Writes `write` in one transaction. Existing rows win: a Profile the account has keeps its
/// fields, a Snapshot it has is skipped, an existing custom price or setting stays. Imported
/// Snapshots carry totals only (`body` NULL) and `imported` 1, so they are never uploaded.
pub async fn write(pool: &SqlitePool, write: &Write<'_>) -> sqlx::Result<Imported> {
    let account_id = write.account_id;
    let mut tx = pool.begin().await?;
    let mut profiles = 0;
    for (position, profile) in write.new_profiles {
        let tabs = db::json_text(&profile.stash_tab_ids)?;
        profiles += sqlx::query(
            "INSERT INTO profiles (account_id, id, position, name, league_id, price_league_id, \
             stash_tab_ids, character_name, include_equipment, include_inventory) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT (account_id, id) DO NOTHING",
        )
        .bind(account_id)
        .bind(&profile.id)
        .bind(position)
        .bind(&profile.name)
        .bind(&profile.league_id)
        .bind(&profile.price_league_id)
        .bind(tabs)
        .bind(&profile.character_name)
        .bind(profile.include_equipment)
        .bind(profile.include_inventory)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    if let Some(active) = write.active_profile_id {
        sqlx::query("UPDATE accounts SET active_profile_id = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(active)
            .execute(&mut *tx)
            .await?;
    }
    let mut added = Vec::new();
    for profile in &write.selection.profiles {
        for snapshot in &profile.snapshots {
            let inserted = sqlx::query(
                "INSERT INTO snapshots \
                 (account_id, id, profile_id, created_at, total_value, body, imported) \
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, 1) ON CONFLICT (account_id, id) DO NOTHING",
            )
            .bind(account_id)
            .bind(&snapshot.id)
            .bind(&profile.id)
            .bind(snapshot.created_at)
            .bind(snapshot.total_value())
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted == 0 {
                continue;
            }
            let tabs: Vec<(&str, &str, f64)> = snapshot
                .tabs
                .iter()
                .map(|tab| {
                    let name = write.selection.tab_name(&tab.tab_id);
                    (tab.tab_id.as_str(), name, tab.value)
                })
                .collect();
            snapshot::insert_tabs(&mut tx, account_id, &snapshot.id, &tabs).await?;
            added.push(snapshot.id.as_str());
        }
        snapshot::retain(&mut tx, account_id, &profile.id).await?;
    }
    let added = db::json_text(&added)?;
    let snapshots: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM snapshots WHERE account_id = ?1 \
         AND id IN (SELECT value FROM json_each(?2))",
    )
    .bind(account_id)
    .bind(added)
    .fetch_one(&mut *tx)
    .await?;
    for price in &write.scan.custom_prices {
        // The unique index `custom_prices_entry` keeps an existing custom price.
        sqlx::query(
            "INSERT INTO custom_prices (price_league_id, name, icon, frame_type, variant, links, \
             quality, level, ilvl, tier, corrupted, elder, shaper, value) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
             ON CONFLICT DO NOTHING",
        )
        .bind(&price.price_league_id)
        .bind(&price.name)
        .bind(&price.icon)
        .bind(price.frame_type)
        .bind(&price.variant)
        .bind(price.links)
        .bind(price.quality)
        .bind(price.level)
        .bind(price.ilvl)
        .bind(price.tier)
        .bind(price.corrupted)
        .bind(price.elder)
        .bind(price.shaper)
        .bind(price.value)
        .execute(&mut *tx)
        .await?;
    }
    for (key, value) in &write.scan.settings {
        if let Err(error) = settings::check(key, value) {
            log::warn!("not importing the legacy setting {key}: {error}");
            continue;
        }
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO NOTHING",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO legacy_import (account_id, state, at) VALUES (?1, 'done', ?2) \
         ON CONFLICT (account_id) DO UPDATE SET state = excluded.state, at = excluded.at",
    )
    .bind(account_id)
    .bind(write.at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Imported {
        profiles: u32::try_from(profiles).unwrap_or(u32::MAX),
        snapshots: u32::try_from(snapshots).unwrap_or(u32::MAX),
    })
}
