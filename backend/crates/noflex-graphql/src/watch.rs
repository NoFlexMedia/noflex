// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::{Context, Enum, Object, SimpleObject};
use noflex_core::{MediaFileId, WatchRoomId};
use noflex_storage::{
    Storage,
    domain::{WatchRoom, WatchRoomMember, WatchRoomState},
};
use time::OffsetDateTime;
use tokio::sync::broadcast;

use crate::access;

#[derive(Clone)]
pub struct WatchRoomBus {
    sender: broadcast::Sender<WatchRoomEvent>,
}
impl Default for WatchRoomBus {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(128);
        Self { sender }
    }
}
impl WatchRoomBus {
    fn send(&self, event: WatchRoomEvent) {
        let _ = self.sender.send(event);
    }
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<WatchRoomEvent> {
        self.sender.subscribe()
    }
}

#[derive(Enum, Clone, Copy, Eq, PartialEq)]
pub enum WatchRoomAction {
    Play,
    Pause,
    Seek,
    Stop,
}
#[derive(Clone, SimpleObject)]
pub struct WatchRoomEvent {
    pub room: WatchRoom,
    pub action: WatchRoomAction,
    pub sequence: u64,
    pub server_timestamp: OffsetDateTime,
}

/// Whether `user` currently belongs to `room`.
pub(crate) async fn is_member(
    storage: &Arc<dyn Storage>,
    room: WatchRoomId,
    user: noflex_core::UserId,
) -> async_graphql::Result<bool> {
    Ok(storage
        .list_watch_room_members(room)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?
        .iter()
        .any(|m| m.user_id == user))
}

#[derive(Default)]
pub struct WatchQuery;
#[Object]
impl WatchQuery {
    async fn watch_rooms(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<WatchRoom>> {
        let user = access::user(ctx)?;
        let s = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        s.list_watch_rooms(user)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
    async fn watch_room(
        &self,
        ctx: &Context<'_>,
        id: WatchRoomId,
    ) -> async_graphql::Result<Option<WatchRoom>> {
        let user = access::user(ctx)?;
        let s = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        if !is_member(s, id, user).await? {
            return Ok(None);
        }
        s.get_watch_room(id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }
}

#[derive(Default)]
pub struct WatchMutation;
#[Object]
impl WatchMutation {
    async fn create_watch_room(
        &self,
        ctx: &Context<'_>,
        media_file_id: MediaFileId,
    ) -> async_graphql::Result<WatchRoom> {
        let p = access::request_user(ctx)?;
        let s = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let media = media_file_id;
        if !access::can_media(s, p, media).await? {
            return Err(async_graphql::Error::new("media file not found"));
        }
        let now = OffsetDateTime::now_utc();
        let room = WatchRoom {
            id: WatchRoomId::new(),
            host_user_id: p.user_id,
            media_file_id: media,
            state: WatchRoomState::Waiting,
            position_ms: 0,
            sequence: 0,
            created_at: now,
            updated_at: now,
        };
        s.create_watch_room(&room)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(room)
    }
    async fn join_watch_room(
        &self,
        ctx: &Context<'_>,
        room_id: WatchRoomId,
    ) -> async_graphql::Result<WatchRoom> {
        let p = access::request_user(ctx)?;
        let s = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let id = room_id;
        s.join_watch_room(&WatchRoomMember {
            room_id: id,
            user_id: p.user_id,
            joined_at: OffsetDateTime::now_utc(),
        })
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?
        .ok_or_else(|| async_graphql::Error::new("watch room not found"))
    }
    async fn leave_watch_room(
        &self,
        ctx: &Context<'_>,
        room_id: WatchRoomId,
    ) -> async_graphql::Result<bool> {
        let user = access::user(ctx)?;
        let s = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let id = room_id;
        if !is_member(s, id, user).await? {
            return Err(async_graphql::Error::new("watch room not found"));
        }
        let ended = s
            .leave_watch_room(id, user)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if let Some(room) = ended {
            emit(ctx, room, WatchRoomAction::Stop)?;
        }
        Ok(true)
    }
    async fn control_watch_room(
        &self,
        ctx: &Context<'_>,
        room_id: WatchRoomId,
        action: WatchRoomAction,
        position_ms: Option<u64>,
    ) -> async_graphql::Result<WatchRoom> {
        let user = access::user(ctx)?;
        let s = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let id = room_id;
        let state = match action {
            WatchRoomAction::Play => Some(WatchRoomState::Playing),
            WatchRoomAction::Pause => Some(WatchRoomState::Paused),
            WatchRoomAction::Seek => None,
            WatchRoomAction::Stop => Some(WatchRoomState::Ended),
        };
        let room = s
            .control_watch_room(id, user, state, position_ms, OffsetDateTime::now_utc())
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| async_graphql::Error::new("watch room not found"))?;
        emit(ctx, room.clone(), action)?;
        Ok(room)
    }
}

fn emit(ctx: &Context<'_>, room: WatchRoom, action: WatchRoomAction) -> async_graphql::Result<()> {
    let sequence = room.sequence;
    ctx.data::<Arc<WatchRoomBus>>()
        .map_err(|_| async_graphql::Error::new("watch room bus not configured"))?
        .send(WatchRoomEvent {
            room,
            action,
            sequence,
            server_timestamp: OffsetDateTime::now_utc(),
        });
    Ok(())
}
