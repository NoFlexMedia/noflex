// SPDX-License-Identifier: AGPL-3.0-only
//
// `activeStreams` is admin-only and resolves the service's records to
// users and works, dropping streams whose user is gone.

use std::sync::Arc;

use noflex_core::{MediaFileId, UserId, UserRole};
use noflex_graphql::{
    ActiveStreamRecord, PlaybackSession, PlaybackSessionService, SchemaContext, TranscodeProgress,
    build_schema,
};
use noflex_storage::{RequestUser, Storage, domain::User};
use noflex_storage_turso::TursoStore;
use noflex_transcoder::{
    AudioPlan, Encoder, HlsSegmentContainer, PlaybackDecisionInfo, TranscodePlan, VideoPlan,
};
use time::OffsetDateTime;

struct Streams(Vec<ActiveStreamRecord>);

#[async_trait::async_trait]
impl PlaybackSessionService for Streams {
    async fn start(
        &self,
        _: UserId,
        _: Option<noflex_core::SessionId>,
        _: noflex_graphql::PlaybackStart,
    ) -> anyhow::Result<PlaybackSession> {
        anyhow::bail!("unused")
    }

    async fn stop(&self, _: UserId, _: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn active_streams(&self) -> anyhow::Result<Vec<ActiveStreamRecord>> {
        Ok(self.0.clone())
    }
}

fn record(user: UserId, id: &str) -> ActiveStreamRecord {
    let plan = TranscodePlan {
        segment_container: HlsSegmentContainer::Ts,
        video: VideoPlan::Encode {
            codec: noflex_core::VideoCodec::H264,
            width: 1280,
            height: 720,
            tonemap: false,
            profile: noflex_core::VideoProfile::H264High,
        },
        audio: AudioPlan::None,
    };
    let now = OffsetDateTime::now_utc();
    ActiveStreamRecord {
        id: id.to_string(),
        user_id: user,
        work_id: None,
        media_file_id: MediaFileId::new(),
        channel_id: None,
        device: Some("Firefox".into()),
        started_at: now,
        last_active_at: now,
        position_ms: Some(1_000),
        decision: PlaybackDecisionInfo::live(&plan, Encoder::Vaapi264, None),
        transcode: Some(TranscodeProgress {
            job_id: async_graphql::ID(id.to_string()),
            status: "running".into(),
            encoder: "h264_vaapi".into(),
            hardware: true,
            fps: Some(48.0),
            speed: Some(2.0),
            out_time_ms: Some(12_000),
            error: None,
        }),
    }
}

async fn user(store: &TursoStore, name: &str, admin: bool) -> User {
    let user = User {
        id: UserId::new(),
        name: name.into(),
        password_hash: String::new(),
        is_admin: admin,
        role: if admin {
            UserRole::Admin
        } else {
            UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
    };
    store.upsert_user(&user).await.expect("user");
    user
}

const QUERY: &str = "{ activeStreams { id user { name } work { title } device positionMs \
    decision { plan reasons { code } encoder hardware } \
    transcode { jobId status encoder hardware fps speed outTimeMs } } }";

#[tokio::test]
async fn active_streams_is_admin_only_and_resolves_users() {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let admin = user(&store, "root", true).await;
    let viewer = user(&store, "viewer", false).await;
    let service = Streams(vec![
        record(viewer.id, "job-a"),
        record(UserId::new(), "orphan"),
    ]);
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store) as Arc<dyn Storage>),
        playback_sessions: Some(Arc::new(service)),
        ..Default::default()
    });

    let resp = schema.execute(QUERY).await;
    assert!(!resp.errors.is_empty(), "anonymous must be refused");

    let as_viewer = async_graphql::Request::new(QUERY).data(RequestUser {
        user_id: viewer.id,
        is_admin: false,
    });
    let resp = schema.execute(as_viewer).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "administrator required"),
        "{:?}",
        resp.errors
    );

    let as_admin = async_graphql::Request::new(QUERY).data(RequestUser {
        user_id: admin.id,
        is_admin: true,
    });
    let resp = schema.execute(as_admin).await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let data = resp.data.into_json().expect("json");
    let streams = data["activeStreams"].as_array().expect("list");
    assert_eq!(streams.len(), 1, "orphaned stream is dropped");
    let s = &streams[0];
    assert_eq!(s["id"], "job-a");
    assert_eq!(s["user"]["name"], "viewer");
    assert!(s["work"].is_null());
    assert_eq!(s["device"], "Firefox");
    assert_eq!(s["positionMs"], 1000);
    assert_eq!(s["decision"]["plan"], "TRANSCODE_VIDEO");
    assert_eq!(s["decision"]["reasons"][0]["code"], "LIVE_SOURCE");
    assert_eq!(s["decision"]["encoder"], "h264_vaapi");
    assert_eq!(s["transcode"]["status"], "running");
    assert_eq!(s["transcode"]["hardware"], true);
    assert_eq!(s["transcode"]["outTimeMs"], 12000);
}
