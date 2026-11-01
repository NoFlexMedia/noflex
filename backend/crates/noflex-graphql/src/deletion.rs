// SPDX-License-Identifier: AGPL-3.0-only

use std::{io::ErrorKind, sync::Arc};

use async_graphql::{Context, Object};
use noflex_core::{EditionId, MediaFileId, WorkId};
use noflex_storage::{
    Storage,
    domain::{ActivityKind, MediaFile},
};
use time::OffsetDateTime;

use crate::access;

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

async fn media_files_for_work(
    storage: &Arc<dyn Storage>,
    work_id: WorkId,
) -> async_graphql::Result<Vec<MediaFile>> {
    let editions = storage.editions_for_work(work_id).await.map_err(gql_err)?;
    let mut media_files = Vec::new();
    for edition in editions {
        media_files.extend(
            storage
                .media_files_for_edition(edition.id)
                .await
                .map_err(gql_err)?,
        );
    }
    Ok(media_files)
}

async fn remove_files(paths: &[String]) -> async_graphql::Result<()> {
    for path in paths {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(async_graphql::Error::new(format!(
                    "failed to remove file {path}: {e}"
                )));
            }
        }
    }
    Ok(())
}

async fn log_deletion(
    storage: &Arc<dyn Storage>,
    principal: noflex_storage::RequestUser,
    subject: &str,
    paths: &[String],
    delete_file: bool,
) -> async_graphql::Result<()> {
    let actor = principal.user_id.to_string();
    let detail = serde_json::json!({
        "paths": paths,
        "deleteFile": delete_file,
    })
    .to_string();
    match storage
        .append_activity(
            ActivityKind::ItemRemoved,
            Some(&actor),
            Some(subject),
            Some(&detail),
            OffsetDateTime::now_utc(),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(async_graphql::Error::new(e.to_string())),
    }
}

#[derive(Default)]
pub struct DeletionMutation;

#[Object]
impl DeletionMutation {
    /// Delete a work and its editions and media-file rows. Admin only.
    /// With `deleteFile`, also removes every backing file from disk;
    /// without it, the row-only cleanup resurrects on the next library
    /// scan while the files remain on disk and is not media removal.
    async fn delete_work(
        &self,
        ctx: &Context<'_>,
        id: WorkId,
        delete_file: bool,
    ) -> async_graphql::Result<bool> {
        let principal = access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let work_id = id;
        storage
            .get_work(work_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("work not found"))?;
        let paths = media_files_for_work(storage, work_id)
            .await?
            .into_iter()
            .map(|media| media.path)
            .collect::<Vec<_>>();
        if delete_file {
            remove_files(&paths).await?;
        }
        storage.delete_work(work_id).await.map_err(gql_err)?;
        log_deletion(
            storage,
            principal,
            &work_id.to_string(),
            &paths,
            delete_file,
        )
        .await?;
        Ok(true)
    }

    /// Delete an edition and its media-file rows. Admin only. With
    /// `deleteFile`, also removes every backing file from disk; without
    /// it, the row-only cleanup resurrects on the next library scan while
    /// the files remain on disk and is not media removal.
    async fn delete_edition(
        &self,
        ctx: &Context<'_>,
        id: EditionId,
        delete_file: bool,
    ) -> async_graphql::Result<bool> {
        let principal = access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let edition_id = id;
        storage
            .get_edition(edition_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("edition not found"))?;
        let paths = storage
            .media_files_for_edition(edition_id)
            .await
            .map_err(gql_err)?
            .into_iter()
            .map(|media| media.path)
            .collect::<Vec<_>>();
        if delete_file {
            remove_files(&paths).await?;
        }
        storage.delete_edition(edition_id).await.map_err(gql_err)?;
        log_deletion(
            storage,
            principal,
            &edition_id.to_string(),
            &paths,
            delete_file,
        )
        .await?;
        Ok(true)
    }

    /// Delete a media-file row. Admin only. With `deleteFile`, also
    /// removes its backing file from disk; without it, the row-only
    /// cleanup resurrects on the next library scan while the file remains
    /// on disk and is not media removal.
    async fn delete_media_file(
        &self,
        ctx: &Context<'_>,
        id: MediaFileId,
        delete_file: bool,
    ) -> async_graphql::Result<bool> {
        let principal = access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let media_file_id = id;
        let media_file = storage
            .media_file_by_id(media_file_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("media file not found"))?;
        let paths = vec![media_file.path];
        if delete_file {
            remove_files(&paths).await?;
        }
        storage
            .delete_media_file(media_file_id)
            .await
            .map_err(gql_err)?;
        log_deletion(
            storage,
            principal,
            &media_file_id.to_string(),
            &paths,
            delete_file,
        )
        .await?;
        Ok(true)
    }
}
