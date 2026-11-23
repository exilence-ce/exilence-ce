//! Removing Snapshots (F12, D5): local first, in one transaction under the
//! operation slot, also when the backend cannot be reached (D26). The ids go
//! into `pending_removals` in that transaction; `RemoveSnapshots` then takes
//! the backend's copies, right away, once more after a Group upload of one
//! of them that was in flight, and again after every successful
//! `StartSession` until it succeeds (`client-schema.sql`,
//! client-background-work.md, session start).

use ipc::command::SnapshotEntry;
use ipc::slices::{NotificationKind, Profiles};
use ipc::{IpcError, MessageKey};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use wire::v1::RemoveSnapshotsRequest;

use crate::backend::{self, Backend};
use crate::state::{self, Store};
use crate::{charts, db, group_upload_queue, internal, items, net_worth, slot};

/// `list_snapshots`: the active Profile's Snapshots, newest first; empty
/// while signed out or without an active Profile.
pub async fn list<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<SnapshotEntry>, IpcError> {
    let pool = app.state::<SqlitePool>();
    let account_id: Option<String> = sqlx::query_scalar("SELECT account_id FROM login")
        .fetch_optional(&*pool)
        .await
        .map_err(internal)?;
    let active = app
        .state::<Store>()
        .read(|p: &Profiles| p.active_profile_id.clone());
    let (Some(account_id), Some(profile_id)) = (account_id, active) else {
        return Ok(Vec::new());
    };
    let rows: Vec<(String, i64, f64, bool)> = sqlx::query_as(
        "SELECT id, created_at, total_value, body IS NOT NULL AND imported = 0 FROM snapshots \
         WHERE account_id = ?1 AND profile_id = ?2 \
         ORDER BY created_at DESC, id DESC",
    )
    .bind(&account_id)
    .bind(&profile_id)
    .fetch_all(&*pool)
    .await
    .map_err(internal)?;
    Ok(rows
        .into_iter()
        .map(|(id, created_at, total_value, item_detail)| SnapshotEntry {
            id,
            created_at,
            total_value,
            item_detail,
        })
        .collect())
}

/// `remove_snapshots(snapshot_ids)`: under the operation slot, deletes the
/// signed-in account's Snapshots with these ids and records the uploaded
/// ones in `pending_removals`, in one transaction; they leave the Group
/// upload queue (D27); pushes `net_worth`, `items` and `charts`. Then sends
/// the account's pending removals in the background. A failure shows a
/// toast (F18).
pub async fn remove<R: Runtime>(
    app: &AppHandle<R>,
    snapshot_ids: Vec<String>,
) -> Result<(), IpcError> {
    let done: Result<(), IpcError> = async {
        let _held = slot::try_hold(app)?;
        let pool = app.state::<SqlitePool>();
        let account_id: String = sqlx::query_scalar("SELECT account_id FROM login")
            .fetch_optional(&*pool)
            .await
            .map_err(internal)?
            .ok_or(IpcError::SessionExpired)?;
        let removed = delete(&pool, &account_id, &snapshot_ids)
            .await
            .map_err(internal)?;
        log::info!("{removed} Snapshots removed");
        group_upload_queue::removed(app, &snapshot_ids);
        net_worth::push(app).await;
        items::push(app).await;
        charts::push(app).await;
        Ok(())
    }
    .await;
    if let Err(error) = done {
        log::warn!("removing Snapshots failed: {error}");
        state::push(
            app,
            NotificationKind::Error,
            MessageKey::NotificationSnapshotsRemoveFailed.into(),
            Some(error.message()),
        );
        return Err(error);
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { send_pending(&handle).await });
    Ok(())
}

/// Removes the Snapshots (`client-schema.sql`, Remove Snapshots): the ids of
/// uploaded ones go into `pending_removals`; imported ones were never
/// uploaded (ADR 0002). How many were removed.
async fn delete(pool: &SqlitePool, account_id: &str, snapshot_ids: &[String]) -> sqlx::Result<u64> {
    let ids = db::json_text(snapshot_ids)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO pending_removals (account_id, snapshot_id, profile_id) \
         SELECT account_id, id, profile_id FROM snapshots \
         WHERE account_id = ?1 AND id IN (SELECT value FROM json_each(?2)) AND imported = 0",
    )
    .bind(account_id)
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query(
        "DELETE FROM snapshots WHERE account_id = ?1 AND id IN (SELECT value FROM json_each(?2))",
    )
    .bind(account_id)
    .bind(&ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(removed)
}

/// Sends the signed-in account's pending removals, one `RemoveSnapshots`
/// per Profile, and forgets those the backend took but for Snapshots the
/// Group upload queue still holds (`send`). A failure is only logged: the
/// next successful `StartSession` sends them again.
pub async fn send_pending<R: Runtime>(app: &AppHandle<R>) {
    let pool = app.state::<SqlitePool>();
    let pending: sqlx::Result<_> = async {
        let login: Option<(String, String)> =
            sqlx::query_as("SELECT account_id, session_token FROM login")
                .fetch_optional(&*pool)
                .await?;
        let Some((account_id, session_token)) = login else {
            return Ok(None);
        };
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT profile_id, snapshot_id FROM pending_removals WHERE account_id = ?1 \
             ORDER BY profile_id, snapshot_id",
        )
        .bind(&account_id)
        .fetch_all(&*pool)
        .await?;
        Ok(Some((account_id, session_token, rows)))
    }
    .await;
    let (account_id, session_token, rows) = match pending {
        Ok(Some(pending)) => pending,
        Ok(None) => return,
        Err(error) => return log::error!("cannot read the pending removals: {error}"),
    };
    for profile_rows in rows.chunk_by(|a, b| a.0 == b.0) {
        let request = RemoveSnapshotsRequest {
            profile_id: profile_rows[0].0.clone(),
            snapshot_ids: profile_rows.iter().map(|(_, id)| id.clone()).collect(),
        };
        if let Err(error) = send(app, &pool, &account_id, &session_token, request).await {
            // The backend is unreachable or refuses this Login: the rest
            // waits for the next session start too.
            return log::info!(
                "RemoveSnapshots failed ({error}); sent again after the next session start"
            );
        }
    }
}

/// One `RemoveSnapshots`; on success the sent rows leave `pending_removals`,
/// but for Snapshots the Group upload queue holds when it is sent: their
/// upload may land after it, so their removal goes again (D5,
/// [`group_upload_queue`]).
async fn send<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    account_id: &str,
    session_token: &str,
    request: RemoveSnapshotsRequest,
) -> Result<(), IpcError> {
    let backend = app.state::<Backend>();
    backend.check_superseded()?;
    let profile_id = request.profile_id.clone();
    let forgotten: Vec<&String> = request
        .snapshot_ids
        .iter()
        .filter(|id| !group_upload_queue::holds(app, id))
        .collect();
    let ids = db::json_text(&forgotten).map_err(internal)?;
    let count = request.snapshot_ids.len();
    let call = backend::request("RemoveSnapshots", request, Some(session_token));
    if let Err(status) = backend.profile().remove_snapshots(call).await {
        return Err(backend::failure(app, &status));
    }
    log::debug!("backend copies of {count} Snapshots of profile {profile_id} removed");
    sqlx::query(
        "DELETE FROM pending_removals \
         WHERE account_id = ?1 AND snapshot_id IN (SELECT value FROM json_each(?2))",
    )
    .bind(account_id)
    .bind(ids)
    .execute(pool)
    .await
    .map_err(internal)?;
    Ok(())
}
