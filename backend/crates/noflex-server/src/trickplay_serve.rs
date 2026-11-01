// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::Path, sync::Arc};

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use noflex_core::EditionId;
use noflex_storage::Storage;
use tokio::io::AsyncReadExt;

use crate::file_serve::{OpenedFile, RootCapability, ServeOpts, serve_file};

pub fn parse_sheet_index(raw: &str) -> Option<u32> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "trickplay not found").into_response()
}

pub async fn serve_vtt(
    storage: &Arc<dyn Storage>,
    cache_root: &Path,
    edition: EditionId,
    range_header: Option<&str>,
    token: Option<&str>,
) -> Response {
    let asset = match storage.get_trickplay(edition).await {
        Ok(Some(asset)) => asset,
        Ok(None) => return not_found(),
        Err(error) => {
            tracing::error!(%error, %edition, "storage error reading trickplay manifest");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };
    let Some(opened) = open_asset(cache_root, edition, &asset.vtt_path, "index.vtt").await else {
        return not_found();
    };
    if let Some(token) = token {
        let path = opened.path().to_path_buf();
        let mut body = String::new();
        let mut file = opened.into_file();
        if let Err(error) = file.read_to_string(&mut body).await {
            tracing::error!(%error, path = %path.display(), "read trickplay VTT failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
        }
        let body = rewrite_vtt_token(&body, token);
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/vtt"),
        );
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=3600"),
        );
        return (headers, body).into_response();
    }
    serve_file(
        opened,
        range_header,
        ServeOpts {
            content_disposition: None,
            cache_control: Some("private, max-age=3600"),
        },
    )
    .await
}

pub fn rewrite_vtt_token(body: &str, token: &str) -> String {
    let token = percent_encode(token);
    body.split_inclusive('\n')
        .map(|line| {
            let Some(fragment) = line.find("#xywh=") else {
                return line.to_string();
            };
            let separator = if line[..fragment].contains('?') {
                '&'
            } else {
                '?'
            };
            format!(
                "{}{separator}token={token}{}",
                &line[..fragment],
                &line[fragment..]
            )
        })
        .collect()
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub async fn serve_sheet(
    storage: &Arc<dyn Storage>,
    cache_root: &Path,
    edition: EditionId,
    index: u32,
    range_header: Option<&str>,
) -> Response {
    let asset = match storage.get_trickplay(edition).await {
        Ok(Some(asset)) => asset,
        Ok(None) => return not_found(),
        Err(error) => {
            tracing::error!(%error, %edition, "storage error reading trickplay manifest");
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };
    let Some(sheet) = asset.sheets.iter().find(|sheet| sheet.index == index) else {
        return not_found();
    };
    let expected_name = format!("sheet-{index:04}.jpg");
    let Some(opened) = open_asset(cache_root, edition, &sheet.path, &expected_name).await else {
        return not_found();
    };
    serve_file(
        opened,
        range_header,
        ServeOpts {
            content_disposition: None,
            cache_control: Some("private, max-age=86400"),
        },
    )
    .await
}

async fn open_asset(
    cache_root: &Path,
    edition: EditionId,
    raw_path: &str,
    expected_name: &str,
) -> Option<OpenedFile> {
    let capability = RootCapability::new(cache_root).await.ok()?;
    let opened = capability.open(raw_path).await.ok()?;
    let relative = opened.path().strip_prefix(capability.path()).ok()?;
    let mut components = relative.components();
    let edition_component = components.next()?;
    let version = components.next()?;
    let file = components.next()?;
    let edition_name = edition.to_string();
    if components.next().is_some()
        || !matches!(edition_component, std::path::Component::Normal(name) if name == std::ffi::OsStr::new(&edition_name))
        || !matches!(version, std::path::Component::Normal(_))
        || !matches!(file, std::path::Component::Normal(name) if name == expected_name)
    {
        return None;
    }
    Some(opened)
}
