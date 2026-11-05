// SPDX-License-Identifier: AGPL-3.0-only
//
// HTTP serve path for completed export jobs.
// The `download_file` axum handler in `main.rs` is a thin
// wrapper over `serve_download_job`, which the integration
// tests in `tests/it/download.rs` call directly with a
// `&Arc<dyn Storage>`.

use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use noflex_auth::Principal;
use noflex_core::ExportJobId;
use noflex_storage::Storage;

use crate::file_serve::{RootCapability, ServeOpts, serve_file};

pub async fn serve_download_job(
    storage: &Arc<dyn Storage>,
    principal: &Principal,
    job_id: ExportJobId,
    download_root: &RootCapability,
    range_header: Option<&str>,
) -> Response {
    if !principal.is_authenticated() {
        return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    }

    match storage.downloads_allowed().await {
        Ok(true) => {}
        Ok(false) => {
            return (StatusCode::FORBIDDEN, "this server doesn't allow downloads").into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "storage error reading download setting");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    }

    let job = match storage.get_export_job(job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => return (StatusCode::NOT_FOUND, "job not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "storage error reading job");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };

    // A non-owner sees the same 404 as a missing job, so job
    // existence doesn't leak — same rule as playlist access.
    if !principal.is_admin && job.user_id != principal.user_id {
        return (StatusCode::NOT_FOUND, "job not found").into_response();
    }

    if job.status != noflex_storage::domain::ExportJobStatus::Completed {
        return (
            StatusCode::CONFLICT,
            format!("job not completed (status: {:?})", job.status),
        )
            .into_response();
    }

    let output_path = match job.output_path.as_ref() {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "completed job has no output path",
            )
                .into_response();
        }
    };
    let opened = match download_root.open(&output_path).await {
        Ok(opened) => opened,
        Err(error) => {
            tracing::warn!(%error, path = %output_path.display(), "rejected download output");
            return (StatusCode::NOT_FOUND, "output file not found").into_response();
        }
    };

    let filename = output_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    serve_file(
        opened,
        range_header,
        ServeOpts {
            content_disposition: Some(format!("attachment; filename=\"{filename}\"")),
            cache_control: None,
        },
    )
    .await
}
