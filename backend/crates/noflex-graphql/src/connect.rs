// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex Connect: one signed-in client controlling another, like casting
// between a phone and a TV. A device that can be controlled subscribes to
// `deviceCommands`; anything of the same viewer's sends it commands and sees
// what it reports back. All of it is in memory: a command is for a device
// that is listening now, so none is queued and nothing survives a restart.

use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_graphql::{
    ComplexObject, Context, Enum, ErrorExtensions, InputObject, Object, SimpleObject,
};
use futures::Stream;
use noflex_core::{DeviceId, EditionId, MediaFileId, UserId, WorkId};
use noflex_storage::{
    Storage,
    domain::{Device, Work},
};
use time::OffsetDateTime;
use tokio::sync::{broadcast, mpsc};

use crate::{
    access,
    devices::{gql_err, owned_device},
    users::authed_principal,
};

/// How long a device counts as there after it last said so. Receivers report
/// every few seconds; a TV unplugged mid-stream never closes its socket, so
/// silence is what ends its presence.
const PRESENCE: Duration = Duration::from_secs(40);

/// Commands that may wait for a receiver that is busy starting a title.
const QUEUE: usize = 16;

/// How often a watcher is told again, so a device that went quiet goes offline
/// on their screen without anything else happening.
const REFRESH: Duration = Duration::from_secs(15);

/// What a device is doing, as it last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DevicePlayState {
    Playing,
    Paused,
    Buffering,
}

/// Something a controlling client asks a device to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DeviceCommandKind {
    /// Start a title: the device picks the version it plays best.
    Load,
    Play,
    Pause,
    Seek,
    Stop,
}

/// What `controlDevice` may ask of a device that already has something loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DeviceControl {
    Play,
    Pause,
    Seek,
    Stop,
}

impl From<DeviceControl> for DeviceCommandKind {
    fn from(control: DeviceControl) -> Self {
        match control {
            DeviceControl::Play => Self::Play,
            DeviceControl::Pause => Self::Pause,
            DeviceControl::Seek => Self::Seek,
            DeviceControl::Stop => Self::Stop,
        }
    }
}

/// A command for a device to carry out. Only the fields its `kind` uses are set.
#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceCommand {
    pub kind: DeviceCommandKind,
    /// The title to start, on `Load`.
    pub work_id: Option<WorkId>,
    /// A cut the viewer picked, on `Load`; the device chooses otherwise.
    pub edition_id: Option<EditionId>,
    /// A version the viewer picked, on `Load`; the device chooses otherwise.
    pub media_file_id: Option<MediaFileId>,
    /// Where to start on `Load`, or the place to go on `Seek`.
    pub position_ms: Option<u64>,
    pub issued_at: OffsetDateTime,
}

/// What a device is playing. It's the device's own word, refreshed every few
/// seconds, so a controller advances `positionMs` itself while `PLAYING`.
#[derive(Debug, Clone, PartialEq, SimpleObject)]
#[graphql(complex)]
pub struct DevicePlayback {
    pub work_id: WorkId,
    pub state: DevicePlayState,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    /// The server's clock when it heard, so a controller can tell how stale
    /// `positionMs` is without trusting its own clock.
    pub reported_at: OffsetDateTime,
}

#[ComplexObject]
impl DevicePlayback {
    /// The title; null once it's gone, or for a viewer who can't see it.
    #[graphql(complexity = 3)]
    async fn work(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<Work>> {
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        match storage.get_work(self.work_id).await.map_err(gql_err)? {
            Some(work) => access::visible_work(storage, user, work).await,
            None => Ok(None),
        }
    }
}

/// A device of the caller's and whether it can be controlled right now.
#[derive(Debug, Clone, SimpleObject)]
pub struct ConnectDevice {
    pub device: Device,
    /// Listening for commands, and heard from lately.
    pub online: bool,
    /// Null while it's idle.
    pub playback: Option<DevicePlayback>,
}

/// What a device reports about itself.
#[derive(Debug, Clone, InputObject)]
pub struct DevicePlaybackInput {
    pub work_id: WorkId,
    pub state: DevicePlayState,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
}

/// Why a command didn't reach its device.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SendError {
    Offline,
    Busy,
}

struct Receiver {
    generation: u64,
    commands: mpsc::Sender<DeviceCommand>,
}

struct Presence {
    user: UserId,
    receiver: Option<Receiver>,
    heard: Instant,
    playback: Option<DevicePlayback>,
}

impl Presence {
    fn online(&self, within: Duration) -> bool {
        self.receiver.is_some() && self.heard.elapsed() < within
    }
}

/// Who is listening and what they last said. One per server.
pub struct ConnectHub {
    devices: Mutex<HashMap<DeviceId, Presence>>,
    generations: AtomicU64,
    changed: broadcast::Sender<UserId>,
    presence: Duration,
}

impl Default for ConnectHub {
    fn default() -> Self {
        Self::with_presence(PRESENCE)
    }
}

/// A device's end of its command stream. Dropping it, or a newer attachment of
/// the same device, ends the device's presence.
pub(crate) struct Attachment {
    hub: Arc<ConnectHub>,
    device: DeviceId,
    generation: u64,
    pub(crate) commands: mpsc::Receiver<DeviceCommand>,
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.hub.detach(self.device, self.generation);
    }
}

impl ConnectHub {
    pub(crate) fn with_presence(presence: Duration) -> Self {
        Self {
            devices: Mutex::default(),
            generations: AtomicU64::new(0),
            changed: broadcast::channel(64).0,
            presence,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DeviceId, Presence>> {
        // Nothing here panics while holding the lock, and the map stays valid
        // if something did.
        self.devices.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn announce(&self, user: UserId) {
        let _ = self.changed.send(user);
    }

    /// Whom to tell when one of their devices comes, goes or says something.
    pub(crate) fn watch(&self) -> broadcast::Receiver<UserId> {
        self.changed.subscribe()
    }

    /// Start listening for `device`. A device listens in one place: a second
    /// attachment (the app reconnecting before the old socket is noticed to be
    /// dead) replaces the first, whose stream then ends.
    pub(crate) fn attach(self: &Arc<Self>, user: UserId, device: DeviceId) -> Attachment {
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let (commands, inbox) = mpsc::channel(QUEUE);
        {
            let mut devices = self.lock();
            let entry = devices.entry(device).or_insert_with(|| Presence {
                user,
                receiver: None,
                heard: Instant::now(),
                playback: None,
            });
            entry.user = user;
            entry.heard = Instant::now();
            entry.receiver = Some(Receiver {
                generation,
                commands,
            });
        }
        self.announce(user);
        Attachment {
            hub: self.clone(),
            device,
            generation,
            commands: inbox,
        }
    }

    fn detach(&self, device: DeviceId, generation: u64) {
        let user = {
            let mut devices = self.lock();
            let Some(entry) = devices.get_mut(&device) else {
                return;
            };
            if entry.receiver.as_ref().map(|r| r.generation) != Some(generation) {
                return;
            }
            entry.receiver = None;
            entry.playback = None;
            let user = entry.user;
            devices.remove(&device);
            user
        };
        self.announce(user);
    }

    /// Hand `command` to `device`, which must belong to `user` and be there.
    pub(crate) fn send(
        &self,
        user: UserId,
        device: DeviceId,
        command: DeviceCommand,
    ) -> Result<(), SendError> {
        let devices = self.lock();
        let entry = devices
            .get(&device)
            .filter(|e| e.user == user && e.online(self.presence))
            .ok_or(SendError::Offline)?;
        let receiver = entry.receiver.as_ref().ok_or(SendError::Offline)?;
        receiver.commands.try_send(command).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => SendError::Busy,
            mpsc::error::TrySendError::Closed(_) => SendError::Offline,
        })
    }

    /// Record what `device` says about itself, which also counts as hearing
    /// from it. `None` is idle.
    pub(crate) fn report(&self, user: UserId, device: DeviceId, playback: Option<DevicePlayback>) {
        let quiet = {
            let mut devices = self.lock();
            let entry = devices.entry(device).or_insert_with(|| Presence {
                user,
                receiver: None,
                heard: Instant::now(),
                playback: None,
            });
            let was_online = entry.online(self.presence);
            let quiet = entry.playback.is_none() && playback.is_none() && was_online;
            entry.user = user;
            entry.heard = Instant::now();
            entry.playback = playback;
            quiet
        };
        // An idle device checking in changes nothing anyone is looking at.
        if !quiet {
            self.announce(user);
        }
    }

    /// Forget what `device` was playing, for a stop just sent to it, so every
    /// controller sees it idle now rather than at its next report. A device
    /// that didn't obey says so with its next report.
    pub(crate) fn clear_playback(&self, user: UserId, device: DeviceId) {
        let cleared = {
            let mut devices = self.lock();
            devices
                .get_mut(&device)
                .filter(|entry| entry.user == user)
                .is_some_and(|entry| entry.playback.take().is_some())
        };
        if cleared {
            self.announce(user);
        }
    }

    /// `device`'s presence and what it's playing.
    pub(crate) fn status(&self, user: UserId, device: DeviceId) -> (bool, Option<DevicePlayback>) {
        let devices = self.lock();
        match devices.get(&device).filter(|e| e.user == user) {
            Some(entry) => (
                entry.online(self.presence),
                entry
                    .playback
                    .clone()
                    .filter(|_| entry.heard.elapsed() < self.presence),
            ),
            None => (false, None),
        }
    }
}

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn hub(ctx: &Context<'_>) -> async_graphql::Result<Arc<ConnectHub>> {
    ctx.data::<Arc<ConnectHub>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("connect is not configured"))
}

/// The caller's devices with who is there.
async fn connect_devices(
    storage: &Arc<dyn Storage>,
    hub: &ConnectHub,
    user: UserId,
) -> async_graphql::Result<Vec<ConnectDevice>> {
    let devices = storage.devices_for_user(user).await.map_err(gql_err)?;
    Ok(devices
        .into_iter()
        .map(|device| {
            let (online, playback) = hub.status(user, device.id);
            ConnectDevice {
                device,
                online,
                playback,
            }
        })
        .collect())
}

fn refused(error: SendError) -> async_graphql::Error {
    let (message, code) = match error {
        SendError::Offline => ("That device isn’t available right now.", "DEVICE_OFFLINE"),
        SendError::Busy => ("That device is busy. Try again in a moment.", "DEVICE_BUSY"),
    };
    async_graphql::Error::new(message).extend_with(|_, ext| ext.set("code", code))
}

/// `Subscription.deviceCommands`.
pub(crate) async fn device_commands(
    ctx: &Context<'_>,
    device_id: DeviceId,
) -> async_graphql::Result<Pin<Box<dyn Stream<Item = DeviceCommand> + Send + 'static>>> {
    let principal = authed_principal(ctx)?;
    owned_device(storage(ctx)?, principal.user_id, device_id).await?;
    let mut attachment = hub(ctx)?.attach(principal.user_id, device_id);
    Ok(Box::pin(async_stream::stream! {
        while let Some(command) = attachment.commands.recv().await {
            yield command;
        }
    }))
}

/// `Subscription.connectDevices`: the list now, then again whenever one of the
/// caller's devices comes, goes or says something.
pub(crate) async fn watch_devices(
    ctx: &Context<'_>,
) -> async_graphql::Result<Pin<Box<dyn Stream<Item = Vec<ConnectDevice>> + Send + 'static>>> {
    let user = authed_principal(ctx)?.user_id;
    let storage = storage(ctx)?.clone();
    let hub = hub(ctx)?;
    let mut changes = hub.watch();
    Ok(Box::pin(async_stream::stream! {
        let mut refresh = tokio::time::interval(REFRESH);
        loop {
            tokio::select! {
                _ = refresh.tick() => {}
                changed = changes.recv() => match changed {
                    Ok(who) if who != user => continue,
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
            match connect_devices(&storage, &hub, user).await {
                Ok(devices) => yield devices,
                Err(error) => {
                    tracing::warn!(%user, error = %error.message, "connect: device list failed");
                    break;
                }
            }
        }
    }))
}

#[derive(Default)]
pub struct ConnectQuery;

#[Object]
impl ConnectQuery {
    /// The caller's devices and which can be controlled now.
    async fn connect_devices(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<ConnectDevice>> {
        let user = authed_principal(ctx)?.user_id;
        connect_devices(storage(ctx)?, hub(ctx)?.as_ref(), user).await
    }
}

#[derive(Default)]
pub struct ConnectMutation;

#[Object]
impl ConnectMutation {
    /// Start a title on one of the caller's devices. The device chooses the
    /// version it plays best unless `editionId` or `mediaFileId` say otherwise.
    /// Fails with `DEVICE_OFFLINE` when nothing is listening there.
    async fn cast_to_device(
        &self,
        ctx: &Context<'_>,
        device_id: DeviceId,
        work_id: WorkId,
        edition_id: Option<EditionId>,
        media_file_id: Option<MediaFileId>,
        position_ms: Option<u64>,
    ) -> async_graphql::Result<ConnectDevice> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let device = owned_device(storage, principal.user_id, device_id).await?;
        let user = access::request_user(ctx)?;
        let missing = || async_graphql::Error::new("title not found");
        if !access::can_work(storage, user, work_id).await? {
            return Err(missing());
        }
        if let Some(edition) = edition_id
            && !access::can_edition(storage, user, edition).await?
        {
            return Err(missing());
        }
        if let Some(media) = media_file_id
            && !access::can_media(storage, user, media).await?
        {
            return Err(missing());
        }
        let hub = hub(ctx)?;
        hub.send(
            principal.user_id,
            device_id,
            DeviceCommand {
                kind: DeviceCommandKind::Load,
                work_id: Some(work_id),
                edition_id,
                media_file_id,
                position_ms,
                issued_at: OffsetDateTime::now_utc(),
            },
        )
        .map_err(refused)?;
        let (online, playback) = hub.status(principal.user_id, device_id);
        Ok(ConnectDevice {
            device,
            online,
            playback,
        })
    }

    /// Play, pause, seek or stop what one of the caller's devices has loaded.
    /// `positionMs` is where `SEEK` goes.
    async fn control_device(
        &self,
        ctx: &Context<'_>,
        device_id: DeviceId,
        action: DeviceControl,
        position_ms: Option<u64>,
    ) -> async_graphql::Result<ConnectDevice> {
        let principal = authed_principal(ctx)?;
        let device = owned_device(storage(ctx)?, principal.user_id, device_id).await?;
        if action == DeviceControl::Seek && position_ms.is_none() {
            return Err(async_graphql::Error::new("seeking needs a position"));
        }
        let hub = hub(ctx)?;
        hub.send(
            principal.user_id,
            device_id,
            DeviceCommand {
                kind: action.into(),
                work_id: None,
                edition_id: None,
                media_file_id: None,
                position_ms,
                issued_at: OffsetDateTime::now_utc(),
            },
        )
        .map_err(refused)?;
        if action == DeviceControl::Stop {
            hub.clear_playback(principal.user_id, device_id);
        }
        let (online, playback) = hub.status(principal.user_id, device_id);
        Ok(ConnectDevice {
            device,
            online,
            playback,
        })
    }

    /// Say what this device is playing, or `null` when it's idle. A device that
    /// listens for commands calls this every few seconds: it's also how the
    /// server knows the device is still there.
    async fn report_device_state(
        &self,
        ctx: &Context<'_>,
        device_id: DeviceId,
        playback: Option<DevicePlaybackInput>,
    ) -> async_graphql::Result<bool> {
        let principal = authed_principal(ctx)?;
        owned_device(storage(ctx)?, principal.user_id, device_id).await?;
        hub(ctx)?.report(
            principal.user_id,
            device_id,
            playback.map(|p| DevicePlayback {
                work_id: p.work_id,
                state: p.state,
                position_ms: p.position_ms,
                duration_ms: p.duration_ms,
                reported_at: OffsetDateTime::now_utc(),
            }),
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(kind: DeviceCommandKind) -> DeviceCommand {
        DeviceCommand {
            kind,
            work_id: None,
            edition_id: None,
            media_file_id: None,
            position_ms: None,
            issued_at: OffsetDateTime::now_utc(),
        }
    }

    fn playing(work: WorkId) -> DevicePlayback {
        DevicePlayback {
            work_id: work,
            state: DevicePlayState::Playing,
            position_ms: 1_000,
            duration_ms: Some(60_000),
            reported_at: OffsetDateTime::now_utc(),
        }
    }

    #[tokio::test]
    async fn a_command_reaches_only_a_listening_device_of_the_same_viewer() {
        let hub = Arc::new(ConnectHub::default());
        let (owner, stranger, device) = (UserId::new(), UserId::new(), DeviceId::new());

        assert_eq!(
            hub.send(owner, device, command(DeviceCommandKind::Pause)),
            Err(SendError::Offline),
            "nothing is listening yet"
        );
        let mut attachment = hub.attach(owner, device);
        assert_eq!(
            hub.send(stranger, device, command(DeviceCommandKind::Pause)),
            Err(SendError::Offline),
            "another viewer's command is as good as no device"
        );
        hub.send(owner, device, command(DeviceCommandKind::Pause))
            .expect("owner's command is delivered");
        assert_eq!(
            attachment.commands.recv().await.map(|c| c.kind),
            Some(DeviceCommandKind::Pause)
        );
    }

    #[tokio::test]
    async fn a_reconnecting_device_replaces_its_stale_listener() {
        let hub = Arc::new(ConnectHub::default());
        let (user, device) = (UserId::new(), DeviceId::new());

        let mut stale = hub.attach(user, device);
        let mut fresh = hub.attach(user, device);
        assert!(stale.commands.recv().await.is_none(), "the old stream ends");
        drop(stale);
        assert!(
            hub.status(user, device).0,
            "the old stream going away isn't the device leaving"
        );

        hub.send(user, device, command(DeviceCommandKind::Stop))
            .expect("delivered to the new listener");
        assert_eq!(
            fresh.commands.recv().await.map(|c| c.kind),
            Some(DeviceCommandKind::Stop)
        );
        drop(fresh);
        assert_eq!(hub.status(user, device), (false, None));
    }

    #[tokio::test]
    async fn a_device_that_stops_reporting_goes_offline_and_forgets_what_it_played() {
        let hub = Arc::new(ConnectHub::with_presence(Duration::ZERO));
        let (user, device) = (UserId::new(), DeviceId::new());
        let _attachment = hub.attach(user, device);
        hub.report(user, device, Some(playing(WorkId::new())));

        assert_eq!(hub.status(user, device), (false, None));
        assert_eq!(
            hub.send(user, device, command(DeviceCommandKind::Play)),
            Err(SendError::Offline)
        );
    }

    #[tokio::test]
    async fn a_stop_leaves_the_device_idle_at_once_and_is_announced() {
        let hub = Arc::new(ConnectHub::default());
        let (user, device) = (UserId::new(), DeviceId::new());
        let _attachment = hub.attach(user, device);
        hub.report(user, device, Some(playing(WorkId::new())));
        let mut heard = hub.watch();

        hub.clear_playback(user, device);
        assert_eq!(heard.recv().await.expect("announced"), user);
        assert_eq!(hub.status(user, device), (true, None));
        hub.clear_playback(user, device);
        assert!(
            heard.try_recv().is_err(),
            "nothing left to clear, nothing to say"
        );
        let stranger = UserId::new();
        hub.report(user, device, Some(playing(WorkId::new())));
        hub.clear_playback(stranger, device);
        assert!(
            hub.status(user, device).1.is_some(),
            "only the owner's stop counts"
        );
    }

    #[tokio::test]
    async fn only_a_change_is_announced() {
        let hub = Arc::new(ConnectHub::default());
        let (user, device) = (UserId::new(), DeviceId::new());
        let mut heard = hub.watch();
        let _attachment = hub.attach(user, device);
        assert_eq!(heard.recv().await.expect("arrival"), user);

        hub.report(user, device, None);
        assert!(heard.try_recv().is_err(), "an idle check-in is silent");
        hub.report(user, device, Some(playing(WorkId::new())));
        assert_eq!(heard.recv().await.expect("playback began"), user);
    }
}
