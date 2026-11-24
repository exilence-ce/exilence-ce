//! `export_csv` (F11): the item table rows the UI shows after its filters,
//! as legacy `exportData` writes them with `export-to-csv`
//! (`exilence-ce-legacy/src/utils/export.utils.ts`): a UTF-8 BOM, the title
//! `Data from YYYY-MM-DD HH:mm`, a header line of legacy's item keys and
//! one comma-separated line per row, saved where the user chooses.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use chrono::{DateTime, Local, TimeZone as _};
use ipc::slices::{ItemRow, ItemTab, Items, NotificationKind};
use ipc::{IpcError, MessageKey};
use tauri::{AppHandle, Manager, Runtime};

use crate::desktop::Desktop;
use crate::internal;
use crate::scheduler::Scheduler;
use crate::state::{self, Store};

/// Line ends as `export-to-csv` writes them.
const EOL: &str = "\r\n";

/// The header line: legacy's priced-item keys for the columns the table
/// holds.
const HEADERS: [&str; 14] = [
    "name",
    "frameType",
    "ilvl",
    "tier",
    "corrupted",
    "links",
    "quality",
    "level",
    "stackSize",
    "calculated",
    "total",
    "tab",
    "icon",
    "detailsUrl",
];

/// `export_csv(row_keys)`: asks where to save the CSV of the `items` rows
/// with these keys, in this order, and saves it there; nothing when the
/// user cancels. A failure is notified.
pub async fn export<R: Runtime>(app: &AppHandle<R>, row_keys: &[String]) -> Result<(), IpcError> {
    let at = local(app.state::<Scheduler>().now());
    let text = app.state::<Store>().read(|items: &Items| {
        let by_key: HashMap<&str, &ItemRow> =
            items.rows.iter().map(|r| (r.key.as_str(), r)).collect();
        let rows: Vec<&ItemRow> = row_keys
            .iter()
            .filter_map(|key| by_key.get(key.as_str()).copied())
            .collect();
        csv(&rows, &items.tabs, at)
    });
    let desktop = app.state::<Arc<dyn Desktop>>().inner().clone();
    let saved = match desktop.save_path(file_name(at)).await {
        Ok(None) => return Ok(()),
        Ok(Some(path)) => {
            tauri::async_runtime::spawn_blocking(move || std::fs::write(&path, text).map(|()| path))
                .await
                .map_err(|e| e.to_string())
                .and_then(|saved| saved.map_err(|e| e.to_string()))
        }
        Err(error) => Err(format!("the save dialog failed: {error}")),
    };
    match saved {
        Ok(path) => {
            log::info!("item table exported to {}", path.display());
            Ok(())
        }
        Err(error) => {
            let error = internal(format!("cannot export the item table: {error}"));
            state::push(
                app,
                NotificationKind::Error,
                MessageKey::NotificationCsvExportFailed.into(),
                Some(error.message()),
            );
            Err(error)
        }
    }
}

/// Unix ms in the system's time zone.
pub fn local(unix_ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(unix_ms)
        .earliest()
        .unwrap_or_default()
}

/// The name the save dialog suggests: `Export_YYYY-MM-DD-HH-mm.csv`.
pub fn file_name(at: DateTime<Local>) -> String {
    format!("Export_{}.csv", at.format("%Y-%m-%d-%H-%M"))
}

/// The file's text: BOM, title, headers, then `rows`.
pub fn csv(rows: &[&ItemRow], tabs: &[ItemTab], at: DateTime<Local>) -> String {
    let mut text = format!(
        "\u{feff}Data from {}\r\n\n{}{EOL}",
        at.format("%Y-%m-%d %H:%M"),
        HEADERS.join(",")
    );
    for row in rows {
        write_quoted(&mut text, &row.name);
        let _ = write!(
            text,
            ",{},{},{},{},{},{},{},{},{},{},",
            row.frame_type,
            row.ilvl,
            row.tier,
            boolean(row.corrupted),
            row.links,
            row.quality,
            row.level,
            row.quantity,
            row.price,
            row.total,
        );
        write_quoted(&mut text, &row.tab_names(tabs));
        text.push(',');
        write_quoted(&mut text, &row.icon);
        text.push(',');
        write_quoted(&mut text, row.ninja_url.as_deref().unwrap_or_default());
        text.push_str(EOL);
    }
    text
}

/// Writes a CSV-quoted text field, doubling inner quotes.
fn write_quoted(out: &mut String, text: &str) {
    out.push('"');
    for (i, part) in text.split('"').enumerate() {
        if i > 0 {
            out.push_str("\"\"");
        }
        out.push_str(part);
    }
    out.push('"');
}

fn boolean(value: bool) -> &'static str {
    if value { "TRUE" } else { "FALSE" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f11_csv_file_name_and_title_carry_the_local_time() {
        let at = Local.with_ymd_and_hms(2026, 3, 7, 9, 5, 0).unwrap();
        assert_eq!(file_name(at), "Export_2026-03-07-09-05.csv");
        assert!(csv(&[], &[], at).starts_with("\u{feff}Data from 2026-03-07 09:05\r\n\n"));
    }
}
