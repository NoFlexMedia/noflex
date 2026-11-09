// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use noflex_core::{PlayTarget, PlayTargetKind, UserId, WorkId};
use noflex_storage::{
    domain::{HistoryPage, HistoryRow, UserData, WatchedWork},
    error::{StorageError, StorageResult},
    query::PageCursor,
};
use time::OffsetDateTime;
use turso::{Connection, Value};
use uuid::Uuid;

use crate::{
    convert::{
        blob_to_uuid, col_blob, col_int, col_int_opt, col_real_opt, col_text, col_text_opt,
        dt_to_text, text_to_dt, uuid_to_blob,
    },
    error::{backend_msg, map_turso},
};

const UPSERT: &str = "INSERT INTO user_data (\
    user_id, target_kind, target_id, played, position_ms, play_count, rating, favorite, last_played\
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
ON CONFLICT(user_id, target_kind, target_id) DO UPDATE SET \
    played = excluded.played, \
    position_ms = excluded.position_ms, \
    play_count = excluded.play_count, \
    rating = excluded.rating, \
    favorite = excluded.favorite, \
    last_played = excluded.last_played";

const GET: &str = "SELECT played, position_ms, play_count, rating, favorite, last_played \
    FROM user_data WHERE user_id = ?1 AND target_kind = ?2 AND target_id = ?3";

const RESUMABLE_NON_EPISODES: &str = "SELECT ud.target_id, ud.last_played \
    FROM user_data ud JOIN works w ON w.id = ud.target_id \
    WHERE ud.user_id = ?1 AND ud.target_kind = 'work' AND ud.played = 0 AND ud.position_ms > 0 \
        AND ud.last_played IS NOT NULL AND w.kind <> 'episode'";

const EPISODE_ACTIVITY: &str = "SELECT e.id, season.parent_id, e.parent_index, e.index_number, \
    e.index_end, ud.played, ud.last_played \
    FROM user_data ud \
    JOIN works e ON e.id = ud.target_id \
    JOIN works season ON season.id = e.parent_id \
    WHERE ud.user_id = ?1 AND ud.target_kind = 'work' AND e.kind = 'episode' \
        AND ud.last_played IS NOT NULL AND season.parent_id IS NOT NULL \
        AND (ud.played = 1 OR ud.position_ms > 0)";

const NEXT_EPISODE: &str = "SELECT e.id, e.added_at \
    FROM works e \
    JOIN works season ON season.id = e.parent_id \
    LEFT JOIN user_data ud ON ud.user_id = ?1 AND ud.target_kind = 'work' AND ud.target_id = e.id \
    WHERE season.parent_id = ?2 AND e.kind = 'episode' AND e.parent_index > 0 \
        AND COALESCE(ud.played, 0) = 0 \
        AND (e.parent_index > ?3 OR (e.parent_index = ?3 AND e.index_number > ?4)) \
    ORDER BY e.parent_index, e.index_number LIMIT 1";

const EPISODES_AFTER: &str = "SELECT e.id \
    FROM works e \
    JOIN works season ON season.id = e.parent_id \
    WHERE season.parent_id = ?1 AND e.kind = 'episode' AND e.parent_index > 0 \
        AND (e.parent_index > ?2 OR (e.parent_index = ?2 AND e.index_number > ?3)) \
    ORDER BY e.parent_index, e.index_number LIMIT ?4";

const WATCHED_WORKS: &str = "SELECT w.id, parent.id, grandparent.id, ud.last_played, \
    ud.played, ud.position_ms, ud.favorite, ud.rating \
    FROM user_data ud \
    JOIN works w ON w.id = ud.target_id \
    LEFT JOIN works parent ON parent.id = w.parent_id \
    LEFT JOIN works grandparent ON grandparent.id = parent.parent_id \
    WHERE ud.user_id = ?1 AND ud.target_kind = 'work' \
        AND (ud.played = 1 OR ud.position_ms > 0 OR ud.favorite = 1 OR ud.rating IS NOT NULL)";

/// A series' latest episode activity.
struct LatestEpisode {
    episode: WorkId,
    season: i64,
    /// The last episode number the file covers (`E05-E06` is 6).
    number: i64,
    played: bool,
    at: OffsetDateTime,
}

pub(crate) async fn get(
    conn: &Connection,
    user: UserId,
    target: PlayTarget,
) -> StorageResult<UserData> {
    let (kind, id) = play_target_parts(&target);
    let mut rows = conn
        .query(
            GET,
            vec![
                Value::Blob(uuid_to_blob(user.into_uuid())),
                Value::Text(kind.to_string()),
                Value::Blob(uuid_to_blob(id)),
            ],
        )
        .await
        .map_err(map_turso)?;
    match rows.next().await.map_err(map_turso)? {
        Some(row) => Ok(row_to_user_data(&row)),
        None => Ok(UserData::default()),
    }
}

pub(crate) async fn write(
    conn: &Connection,
    user: UserId,
    target: PlayTarget,
    data: &UserData,
) -> StorageResult<()> {
    let (kind, id) = play_target_parts(&target);
    let params: Vec<Value> = vec![
        Value::Blob(uuid_to_blob(user.into_uuid())),
        Value::Text(kind.to_string()),
        Value::Blob(uuid_to_blob(id)),
        Value::Integer(data.played as i64),
        data.position_ms.map(Value::Integer).unwrap_or(Value::Null),
        Value::Integer(data.play_count as i64),
        data.rating
            .map(|r| Value::Real(r as f64))
            .unwrap_or(Value::Null),
        Value::Integer(data.favorite as i64),
        match data.last_played {
            Some(dt) => Value::Text(crate::convert::dt_to_text(dt)?),
            None => Value::Null,
        },
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn continue_watching(
    conn: &Connection,
    user: UserId,
    active_since: OffsetDateTime,
    limit: u32,
) -> StorageResult<Vec<WorkId>> {
    let user_blob = Value::Blob(uuid_to_blob(user.into_uuid()));
    let mut entries: Vec<(WorkId, OffsetDateTime)> = Vec::new();

    let mut rows = conn
        .query(RESUMABLE_NON_EPISODES, vec![user_blob.clone()])
        .await
        .map_err(map_turso)?;
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let id = blob_to_uuid(col_blob(&row.get_value(0).map_err(map_turso)?)?)?;
        let at = text_to_dt(col_text(&row.get_value(1).map_err(map_turso)?)?)?;
        entries.push((WorkId::from_uuid(id), at));
    }

    for (series, latest) in latest_episode_per_series(conn, &user_blob).await? {
        if !latest.played {
            entries.push((latest.episode, latest.at));
            continue;
        }
        let mut rows = conn
            .query(
                NEXT_EPISODE,
                vec![
                    user_blob.clone(),
                    Value::Blob(uuid_to_blob(series)),
                    Value::Integer(latest.season),
                    Value::Integer(latest.number),
                ],
            )
            .await
            .map_err(map_turso)?;
        if let Some(row) = rows.next().await.map_err(map_turso)? {
            let id = blob_to_uuid(col_blob(&row.get_value(0).map_err(map_turso)?)?)?;
            let added_at = text_to_dt(col_text(&row.get_value(1).map_err(map_turso)?)?)?;
            entries.push((WorkId::from_uuid(id), latest.at.max(added_at)));
        }
    }

    entries.retain(|(_, at)| *at >= active_since);
    entries.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    entries.truncate(limit as usize);
    Ok(entries.into_iter().map(|(id, _)| id).collect())
}

pub(crate) async fn episodes_after(
    conn: &Connection,
    series: WorkId,
    season: i32,
    number: i32,
    limit: u32,
) -> StorageResult<Vec<WorkId>> {
    let mut rows = conn
        .query(
            EPISODES_AFTER,
            vec![
                Value::Blob(uuid_to_blob(series.into_uuid())),
                Value::Integer(season.into()),
                Value::Integer(number.into()),
                Value::Integer(limit.into()),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        out.push(WorkId::from_uuid(blob_to_uuid(col_blob(
            &row.get_value(0).map_err(map_turso)?,
        )?)?));
    }
    Ok(out)
}

async fn latest_episode_per_series(
    conn: &Connection,
    user: &Value,
) -> StorageResult<HashMap<Uuid, LatestEpisode>> {
    let mut rows = conn
        .query(EPISODE_ACTIVITY, vec![user.clone()])
        .await
        .map_err(map_turso)?;
    let mut latest: HashMap<Uuid, LatestEpisode> = HashMap::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let series = blob_to_uuid(col_blob(&row.get_value(1).map_err(map_turso)?)?)?;
        let number = col_int_opt(&row.get_value(3).map_err(map_turso)?)?.unwrap_or(0);
        let episode_end = col_int_opt(&row.get_value(4).map_err(map_turso)?)?;
        let candidate = LatestEpisode {
            episode: WorkId::from_uuid(blob_to_uuid(col_blob(
                &row.get_value(0).map_err(map_turso)?,
            )?)?),
            season: col_int_opt(&row.get_value(2).map_err(map_turso)?)?.unwrap_or(0),
            number: episode_end.unwrap_or(number).max(number),
            played: col_int(&row.get_value(5).map_err(map_turso)?)? != 0,
            at: text_to_dt(col_text(&row.get_value(6).map_err(map_turso)?)?)?,
        };
        // Bulk "mark watched" stamps episodes almost together; on a tie the
        // later episode is the one that was reached.
        let rank = |e: &LatestEpisode| (e.at, e.season, e.number);
        match latest.get(&series) {
            Some(current) if rank(current) >= rank(&candidate) => {}
            _ => {
                latest.insert(series, candidate);
            }
        }
    }
    Ok(latest)
}

pub(crate) async fn watched_top_level_works(
    conn: &Connection,
    user: UserId,
) -> StorageResult<Vec<WatchedWork>> {
    let mut rows = conn
        .query(
            WATCHED_WORKS,
            vec![Value::Blob(uuid_to_blob(user.into_uuid()))],
        )
        .await
        .map_err(map_turso)?;
    let mut watched: HashMap<Uuid, WatchedWork> = HashMap::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        // The highest ancestor present: grandparent, then parent, then self.
        let mut top = Value::Null;
        for column in [2, 1, 0] {
            top = row.get_value(column).map_err(map_turso)?;
            if !matches!(top, Value::Null) {
                break;
            }
        }
        let top = blob_to_uuid(col_blob(&top)?)?;
        let played = col_int(&row.get_value(4).map_err(map_turso)?)? != 0;
        let position_ms = col_int_opt(&row.get_value(5).map_err(map_turso)?)?;
        let favorite = col_int(&row.get_value(6).map_err(map_turso)?)? != 0;
        let rating = col_real_opt(&row.get_value(7).map_err(map_turso)?)?;
        // Only playback counts as activity. Unmarking a title stamps the row
        // too, and a favorite or rating can keep that row selected.
        let last_played = if played || position_ms.is_some_and(|position| position > 0) {
            col_text_opt(&row.get_value(3).map_err(map_turso)?)?
                .map(|text| text_to_dt(&text))
                .transpose()?
        } else {
            None
        };

        let entry = watched.entry(top).or_insert(WatchedWork {
            work: WorkId::from_uuid(top),
            last_played: OffsetDateTime::UNIX_EPOCH,
            finished: false,
            position_ms: None,
            favorite: false,
            rating: None,
        });
        if let Some(at) = last_played {
            entry.last_played = entry.last_played.max(at);
        }
        entry.finished |= played;
        entry.position_ms = entry.position_ms.max(position_ms);
        entry.favorite |= favorite;
        entry.rating = match (entry.rating, rating) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
    let mut out = watched
        .into_values()
        .filter(|entry| entry.last_played != OffsetDateTime::UNIX_EPOCH)
        .collect::<Vec<_>>();
    out.sort_by_key(|watched| std::cmp::Reverse(watched.last_played));
    Ok(out)
}

fn play_target_parts(target: &PlayTarget) -> (&'static str, Uuid) {
    match target {
        PlayTarget::Work(id) => ("work", id.into_uuid()),
        PlayTarget::Edition(id) => ("edition", id.into_uuid()),
    }
}

fn row_to_user_data(row: &turso::Row) -> UserData {
    let played = row.get_value(0).map_err(map_turso).unwrap_or(Value::Null);
    let position = row.get_value(1).map_err(map_turso).unwrap_or(Value::Null);
    let play_count = row.get_value(2).map_err(map_turso).unwrap_or(Value::Null);
    let rating = row.get_value(3).map_err(map_turso).unwrap_or(Value::Null);
    let favorite = row.get_value(4).map_err(map_turso).unwrap_or(Value::Null);
    let last_played = row.get_value(5).map_err(map_turso).unwrap_or(Value::Null);

    UserData {
        played: col_int(&played).unwrap_or(0) != 0,
        position_ms: col_int_opt(&position).unwrap_or(None),
        play_count: col_int(&play_count).unwrap_or(0) as u32,
        rating: col_real_opt(&rating).unwrap_or(None),
        favorite: col_int(&favorite).unwrap_or(0) != 0,
        last_played: match col_text_opt(&last_played).unwrap_or(None) {
            Some(s) => crate::convert::text_to_dt(&s).ok(),
            None => None,
        },
    }
}

// Total order for history paging: newest first, ties broken by target.
const HISTORY: &str = "SELECT target_kind, target_id, played, position_ms, play_count, rating, \
    favorite, last_played FROM user_data \
    WHERE user_id = ?1 AND last_played IS NOT NULL AND (?2 IS NULL OR last_played < ?2 \
        OR (last_played = ?2 AND (target_kind < ?3 OR (target_kind = ?3 AND target_id < ?4)))) \
    ORDER BY last_played DESC, target_kind DESC, target_id DESC LIMIT ?5";

const TIMESTAMP_LEN: usize = "2000-01-01T00:00:00.000000000Z".len();

fn encode_history_cursor(last_played: &str, kind: &str, id: Uuid) -> PageCursor {
    let tag = if kind == "edition" { 'e' } else { 'w' };
    PageCursor(format!("{last_played}{tag}{}", id.simple()))
}

fn decode_history_cursor(cursor: &PageCursor) -> StorageResult<(String, &'static str, Uuid)> {
    let invalid = || StorageError::InvalidInput("invalid play history cursor".into());
    let raw = cursor.0.as_str();
    if !raw.is_ascii() || raw.len() != TIMESTAMP_LEN + 33 {
        return Err(invalid());
    }
    let (timestamp, rest) = raw.split_at(TIMESTAMP_LEN);
    // Only the canonical rendering compares correctly against stored text.
    if text_to_dt(timestamp)
        .ok()
        .map(dt_to_text)
        .transpose()?
        .as_deref()
        != Some(timestamp)
    {
        return Err(invalid());
    }
    let kind = match &rest[..1] {
        "w" => "work",
        "e" => "edition",
        _ => return Err(invalid()),
    };
    let id = Uuid::parse_str(&rest[1..]).map_err(|_| invalid())?;
    Ok((timestamp.to_string(), kind, id))
}

pub(crate) async fn play_history(
    conn: &Connection,
    user: UserId,
    cursor: Option<PageCursor>,
    limit: u32,
) -> StorageResult<HistoryPage> {
    let limit = limit.max(1);
    let after = cursor.as_ref().map(decode_history_cursor).transpose()?;
    let (timestamp, kind, id) = match after {
        Some((timestamp, kind, id)) => (
            Value::Text(timestamp),
            Value::Text(kind.to_string()),
            Value::Blob(uuid_to_blob(id)),
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    let mut rows = conn
        .query(
            HISTORY,
            vec![
                Value::Blob(uuid_to_blob(user.into_uuid())),
                timestamp,
                kind,
                id,
                Value::Integer(i64::from(limit) + 1),
            ],
        )
        .await
        .map_err(map_turso)?;
    let mut page = HistoryPage::default();
    let mut has_more = false;
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        if page.rows.len() == limit as usize {
            has_more = true;
            break;
        }
        let kind_text = col_text(&row.get_value(0).map_err(map_turso)?)?.to_string();
        let target_kind = match kind_text.as_str() {
            "work" => PlayTargetKind::Work,
            "edition" => PlayTargetKind::Edition,
            other => return Err(backend_msg(format!("unknown play target kind {other}"))),
        };
        let target_id = blob_to_uuid(col_blob(&row.get_value(1).map_err(map_turso)?)?)?;
        let last_played_text = col_text(&row.get_value(7).map_err(map_turso)?)?.to_string();
        let last_played = text_to_dt(&last_played_text)?;
        page.rows.push(HistoryRow {
            target_kind,
            target_id,
            data: UserData {
                played: col_int(&row.get_value(2).map_err(map_turso)?)? != 0,
                position_ms: col_int_opt(&row.get_value(3).map_err(map_turso)?)?,
                play_count: u32::try_from(col_int(&row.get_value(4).map_err(map_turso)?)?)
                    .map_err(|_| backend_msg("negative play count"))?,
                rating: col_real_opt(&row.get_value(5).map_err(map_turso)?)?,
                favorite: col_int(&row.get_value(6).map_err(map_turso)?)? != 0,
                last_played: Some(last_played),
            },
            // Re-render so rows written before timestamp normalization
            // still yield a cursor that compares correctly.
            cursor: encode_history_cursor(&dt_to_text(last_played)?, &kind_text, target_id),
        });
    }
    if has_more {
        page.next_cursor = page.rows.last().map(|row| row.cursor.0.clone());
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use noflex_core::UserRole;
    use noflex_storage::{Storage, domain::User};
    use time::{OffsetDateTime, macros::datetime};

    use super::*;
    use crate::TursoStore;

    #[tokio::test]
    async fn history_pages_through_last_played_ties() {
        let store = TursoStore::open_temp().await.unwrap();
        store.migrate().await.unwrap();
        let user = User {
            id: UserId::new(),
            username: "ties".into(),
            display_name: None,
            password_hash: String::new(),
            is_admin: false,
            role: UserRole::User,
            created_at: OffsetDateTime::now_utc(),
            avatar_updated_at: None,
            has_pin: false,
        };
        store.upsert_user(&user).await.unwrap();
        let tie = dt_to_text(datetime!(2030-01-01 00:00 UTC)).unwrap();
        let older = dt_to_text(datetime!(2029-12-31 00:00 UTC)).unwrap();
        let mut expected = Vec::new();
        {
            let writer = store.writers.acquire().await.unwrap();
            for (index, kind) in ["work", "edition", "work", "edition", "work"]
                .into_iter()
                .enumerate()
            {
                let id = Uuid::now_v7();
                let last_played = if index < 4 { &tie } else { &older };
                writer
                    .execute(
                        "INSERT INTO user_data (user_id, target_kind, target_id, played, \
                         play_count, favorite, last_played) VALUES (?1, ?2, ?3, 1, 1, 0, ?4)",
                        vec![
                            Value::Blob(uuid_to_blob(user.id.into_uuid())),
                            Value::Text(kind.into()),
                            Value::Blob(uuid_to_blob(id)),
                            Value::Text(last_played.clone()),
                        ],
                    )
                    .await
                    .unwrap();
                expected.push((index < 4, kind, id));
            }
        }
        // Newest first; ties by target_kind then target_id, descending.
        expected.sort_by(|a, b| b.cmp(a));
        let expected: Vec<_> = expected.into_iter().map(|(_, _, id)| id).collect();

        for limit in 1..=3 {
            let mut seen = Vec::new();
            let mut cursor = None;
            loop {
                let page = store
                    .list_play_history(user.id, cursor, limit)
                    .await
                    .unwrap();
                seen.extend(page.rows.iter().map(|row| row.target_id));
                match page.next_cursor {
                    Some(next) => cursor = Some(PageCursor(next)),
                    None => break,
                }
            }
            assert_eq!(seen, expected, "limit {limit}");
        }
    }
}
