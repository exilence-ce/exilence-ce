//! The `settings` table (`client-schema.sql`): one JSON value per key, read
//! into the `settings` slice. A missing key means its default.

use std::ops::RangeInclusive;

use domain::pricing::PricingOptions;
use ipc::command::SettingsPatch;
use ipc::slices::{ITEM_TABLE_PAGE_SIZES, Settings};
use ipc::{Field, IpcError, MessageKey};
use serde_json::{Map, Value};
use sqlx::SqlitePool;

/// The stored settings. A stored value that does not fit its setting (written
/// by a newer version, or hand-edited) is logged and leaves the default; keys
/// the slice does not hold are ignored.
pub async fn load(pool: &SqlitePool) -> sqlx::Result<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    let mut settings = Settings::default();
    for (key, value) in rows {
        let value = match serde_json::from_str(&value) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("ignoring the stored setting {key}: {error}");
                continue;
            }
        };
        let merged = check(&key, &value).and_then(|()| {
            merge(&mut settings, Map::from_iter([(key.clone(), value)])).map_err(|e| e.to_string())
        });
        if let Err(error) = merged {
            log::warn!("ignoring the stored setting {key}: {error}");
        }
    }
    Ok(settings)
}

/// Whether `value` fits the setting `key`: its type and range ([`validate`]). Keys the slice
/// does not hold fit.
pub fn check(key: &str, value: &Value) -> Result<(), String> {
    let entry = Map::from_iter([(key.to_owned(), value.clone())]);
    let patch = serde_json::from_value::<SettingsPatch>(Value::Object(entry.clone()))
        .map_err(|e| e.to_string())?;
    validate(&patch).map_err(|e| e.to_string())?;
    merge(&mut Settings::default(), entry).map_err(|e| e.to_string())
}

/// The interval of auto-snapshot in minutes (F4).
pub const AUTO_SNAPSHOT_MINUTES: RangeInclusive<u32> = 5..=1000;
/// The per-item price threshold in chaos (F5).
pub const PRICE_THRESHOLD: RangeInclusive<f64> = 0.0..=100.0;
/// The total price threshold in chaos (F5).
pub const TOTAL_PRICE_THRESHOLD: RangeInclusive<f64> = 0.0..=5000.0;
/// The UI scale in percent, in steps of [`UI_SCALE_STEP`] (F17).
pub const UI_SCALE: RangeInclusive<u32> = 50..=150;
pub const UI_SCALE_STEP: u32 = 10;

/// The pricing options (F5) the settings hold now.
pub fn pricing_options(settings: &Settings) -> PricingOptions {
    PricingOptions {
        low_confidence_pricing: settings.low_confidence_pricing,
        price_threshold: settings.price_threshold,
        total_price_threshold: settings.total_price_threshold,
        exempt_lifeforce_and_markers: settings.exempt_lifeforce_and_markers,
    }
}

/// Checks the ranges of the settings `patch` sets (`client-schema.sql`): the
/// first one outside its range fails as a field validation.
pub fn validate(patch: &SettingsPatch) -> Result<(), IpcError> {
    let out_of_range = |field| IpcError::Validation {
        field,
        key: MessageKey::ErrorSettingOutOfRange,
    };
    if patch
        .auto_snapshot_minutes
        .is_some_and(|m| !AUTO_SNAPSHOT_MINUTES.contains(&m))
    {
        return Err(out_of_range(Field::AutoSnapshotMinutes));
    }
    if patch
        .price_threshold
        .is_some_and(|t| !PRICE_THRESHOLD.contains(&t))
    {
        return Err(out_of_range(Field::PriceThreshold));
    }
    if patch
        .total_price_threshold
        .is_some_and(|t| !TOTAL_PRICE_THRESHOLD.contains(&t))
    {
        return Err(out_of_range(Field::TotalPriceThreshold));
    }
    if patch
        .ui_scale
        .is_some_and(|s| !UI_SCALE.contains(&s) || s % UI_SCALE_STEP != 0)
    {
        return Err(out_of_range(Field::UiScale));
    }
    if patch
        .item_table_page_size
        .is_some_and(|s| !ITEM_TABLE_PAGE_SIZES.contains(&s))
    {
        return Err(out_of_range(Field::ItemTablePageSize));
    }
    Ok(())
}

/// Stores every setting `patch` sets, in one transaction, and returns them as
/// key → value. Out-of-range values are refused ([`validate`]); nothing is
/// stored then.
pub async fn save(
    pool: &SqlitePool,
    patch: &SettingsPatch,
) -> Result<Map<String, Value>, IpcError> {
    validate(patch)?;
    let entries = entries(patch);
    store(pool, &entries).await.map_err(crate::internal)?;
    Ok(entries)
}

/// Stores that the tour ended and What's New opened by itself (F19), and
/// returns them as key → value. Not part of [`SettingsPatch`]: only
/// `complete_tour` sets them.
pub async fn save_tour_completed(pool: &SqlitePool) -> sqlx::Result<Map<String, Value>> {
    let entries = Map::from_iter(
        ["tour_completed", "whats_new_shown"].map(|key| (key.to_owned(), Value::Bool(true))),
    );
    store(pool, &entries).await?;
    Ok(entries)
}

async fn store(pool: &SqlitePool, entries: &Map<String, Value>) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    for (key, value) in entries {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// The settings `patch` sets, as key → value.
fn entries(patch: &SettingsPatch) -> Map<String, Value> {
    match serde_json::to_value(patch) {
        Ok(Value::Object(entries)) => entries,
        _ => unreachable!("a SettingsPatch serializes to an object"),
    }
}

/// Sets the settings in `entries` (key → value) on `settings`; keys
/// `settings` does not hold are ignored. On error `settings` is unchanged.
pub fn merge(settings: &mut Settings, entries: Map<String, Value>) -> serde_json::Result<()> {
    let Value::Object(mut merged) = serde_json::to_value(&*settings)? else {
        unreachable!("Settings serializes to an object");
    };
    for (key, value) in entries {
        if let Some(slot) = merged.get_mut(&key) {
            *slot = value;
        }
    }
    *settings = serde_json::from_value(Value::Object(merged))?;
    Ok(())
}
