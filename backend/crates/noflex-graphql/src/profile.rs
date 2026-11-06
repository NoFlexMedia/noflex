// SPDX-License-Identifier: AGPL-3.0-only
//
// What a viewer changes about themselves. Everything else keys on their id, so a
// new username leaves sessions, marks and history where they were.

use std::{io::Cursor, process::Stdio, sync::Arc, time::Duration};

use async_graphql::{Context, Object};
use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, Rgba, RgbaImage, imageops};
use noflex_storage::{Storage, StorageError, domain::User};
use time::OffsetDateTime;

use crate::users::authed_principal;

const MAX_NAME_CHARS: usize = 100;
/// Decoded. Clients scale a photo down first, so this only meets a caller who didn't.
const MAX_PICTURE_BYTES: usize = 8 * 1024 * 1024;
/// A pixel-bomb limit: this much is well past any photo, and costs under a second.
const MAX_PICTURE_SIDE: u32 = 8192;
const MAX_PICTURE_ALLOC: u64 = 256 * 1024 * 1024;
const PICTURE_SIDE: u32 = 512;
const PICTURE_QUALITY: u8 = 85;
/// Enough for the crop that follows, and no more of a phone photo than that.
const FFMPEG_SCALE: &str = "scale=1024:1024:force_original_aspect_ratio=increase";
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(6);
/// ISO base media brands of the still formats `image` can't read: AVIF, HEIC and
/// the general HEIF family they share.
const HEIF_BRANDS: [&[u8; 4]; 7] = [
    b"avif", b"avis", b"heic", b"heix", b"hevc", b"mif1", b"msf1",
];

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

async fn viewer(ctx: &Context<'_>) -> async_graphql::Result<User> {
    let principal = authed_principal(ctx)?;
    storage(ctx)?
        .get_user(principal.user_id)
        .await
        .map_err(gql_err)?
        .ok_or_else(|| async_graphql::Error::new("user not found"))
}

/// Why a picture was refused, for showing to whoever chose it.
#[derive(Debug, thiserror::Error)]
enum PictureError {
    #[error("That picture is too large")]
    TooLarge,
    #[error("That picture can't be read. Try a JPEG, PNG, WebP or AVIF")]
    Unreadable,
}

/// Whether `bytes` is an ISO base media file claiming an AVIF or HEIF brand.
fn is_heif(bytes: &[u8]) -> bool {
    let Some(size) = bytes.get(..4) else {
        return false;
    };
    let size = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
    let Some(header) = bytes.get(8..size.min(bytes.len())) else {
        return false;
    };
    bytes.get(4..8) == Some(b"ftyp")
        && header
            .as_chunks::<4>()
            .0
            .iter()
            .any(|brand| HEIF_BRANDS.iter().any(|known| brand == known.as_slice()))
}

/// The first frame of a HEIF-family picture as a PNG, by way of ffmpeg, which reads
/// grids of tiles and rotation as phones write them. The demuxer is forced and only
/// files allowed, so what's uploaded can't send it elsewhere the way a playlist
/// could. From a file because the demuxer seeks.
async fn decode_with_ffmpeg(bytes: &[u8]) -> Result<Vec<u8>, PictureError> {
    let unreadable = |error: std::io::Error| {
        tracing::warn!(%error, "profile picture not handed to ffmpeg");
        PictureError::Unreadable
    };
    let file = tempfile::NamedTempFile::new().map_err(unreadable)?;
    tokio::fs::write(file.path(), bytes)
        .await
        .map_err(unreadable)?;
    let output =
        tokio::process::Command::new(noflex_core::bundle::ffmpeg_tool("ffmpeg", "FFMPEG_BIN"))
            .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
            .args(["-protocol_whitelist", "file", "-f", "mov", "-i"])
            .arg(file.path())
            .args(["-frames:v", "1", "-vf", FFMPEG_SCALE, "-pix_fmt", "rgb24"])
            .args(["-c:v", "png", "-f", "image2pipe", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
    match tokio::time::timeout(FFMPEG_TIMEOUT, output).await {
        Ok(Ok(output)) if output.status.success() && !output.stdout.is_empty() => Ok(output.stdout),
        Ok(Ok(output)) => {
            tracing::debug!(stderr = %String::from_utf8_lossy(&output.stderr), "profile picture not decoded");
            Err(PictureError::Unreadable)
        }
        Ok(Err(error)) => Err(unreadable(error)),
        Err(_) => {
            tracing::warn!("profile picture took too long to decode");
            Err(PictureError::Unreadable)
        }
    }
}

/// A centred square JPEG of what was uploaded. Encoding afresh rather than keeping
/// the file means nothing but pixels survives: a phone photo's location goes, and
/// nothing a browser might execute gets served from this origin.
fn prepare_picture(bytes: &[u8]) -> Result<Vec<u8>, PictureError> {
    let unreadable = |_| PictureError::Unreadable;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_PICTURE_SIDE);
    limits.max_image_height = Some(MAX_PICTURE_SIDE);
    limits.max_alloc = Some(MAX_PICTURE_ALLOC);
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| PictureError::Unreadable)?;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    let orientation = decoder.orientation().map_err(unreadable)?;
    let mut picture = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    picture.apply_orientation(orientation);
    let square = picture.resize_to_fill(PICTURE_SIDE, PICTURE_SIDE, imageops::FilterType::Lanczos3);
    // JPEG has no transparency, and dark is what's behind a picture on every client.
    let mut flat = RgbaImage::from_pixel(PICTURE_SIDE, PICTURE_SIDE, Rgba([0, 0, 0, 255]));
    imageops::overlay(&mut flat, &square.to_rgba8(), 0, 0);
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(flat)
        .to_rgb8()
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out,
            PICTURE_QUALITY,
        ))
        .map_err(unreadable)?;
    Ok(out)
}

#[derive(Default)]
pub struct ProfileMutation;

#[Object]
impl ProfileMutation {
    /// Set what the caller is called. Blank goes back to their username.
    async fn update_profile(&self, ctx: &Context<'_>, name: String) -> async_graphql::Result<User> {
        let mut user = viewer(ctx).await?;
        let name = name.trim();
        if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
            return Err(async_graphql::Error::new(
                "a name is at most 100 characters, on one line",
            ));
        }
        user.display_name = (!name.is_empty()).then(|| name.to_string());
        storage(ctx)?.upsert_user(&user).await.map_err(gql_err)?;
        Ok(user)
    }

    /// Change what the caller signs in with. Takes their password, as whoever holds
    /// a session shouldn't be able to lock them out; an account that only signs in
    /// through a provider has none, and passes an empty one. Sessions stay signed in.
    // Credential checks cost most of the per-operation complexity budget, so
    // aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn change_username(
        &self,
        ctx: &Context<'_>,
        username: String,
        current_password: String,
    ) -> async_graphql::Result<User> {
        let mut user = viewer(ctx).await?;
        let username = username.trim();
        if username.is_empty()
            || username.len() > noflex_auth::MAX_USERNAME_BYTES
            || username.chars().any(char::is_control)
        {
            return Err(async_graphql::Error::new(
                "choose a username that fits on one line",
            ));
        }
        if current_password.len() > noflex_auth::MAX_PASSWORD_BYTES {
            return Err(async_graphql::Error::new("password is too long"));
        }
        if !user.password_hash.is_empty() {
            let throttle = crate::auth::login_throttle(ctx)?;
            let client_addr = ctx.data_opt::<noflex_auth::ClientAddr>().copied();
            throttle
                .check(&user.username, client_addr)
                .map_err(crate::auth::too_many_attempts)?;
            if noflex_auth::verify_password(&current_password, &user.password_hash)
                .await
                .is_err()
            {
                throttle.record_failure(&user.username, client_addr);
                return Err(async_graphql::Error::new("incorrect current password"));
            }
            throttle.record_success(&user.username);
        }
        user.username = username.to_string();
        storage(ctx)?
            .upsert_user(&user)
            .await
            .map_err(|e| match e {
                StorageError::Constraint(_) => async_graphql::Error::new("that username is taken"),
                other => gql_err(other),
            })?;
        Ok(user)
    }

    /// Set the caller's picture from a base64 JPEG, PNG, WebP, AVIF or HEIC. It's
    /// cropped to a centred square and everything but its pixels is dropped.
    async fn set_profile_picture(
        &self,
        ctx: &Context<'_>,
        image: String,
    ) -> async_graphql::Result<User> {
        let user = viewer(ctx).await?;
        if image.len() > MAX_PICTURE_BYTES / 3 * 4 + 4 {
            return Err(PictureError::TooLarge.to_string().into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image.trim())
            .map_err(|_| PictureError::Unreadable.to_string())?;
        let bytes = if is_heif(&bytes) {
            decode_with_ffmpeg(&bytes)
                .await
                .map_err(|error| error.to_string())?
        } else {
            bytes
        };
        let picture = tokio::task::spawn_blocking(move || prepare_picture(&bytes))
            .await
            .map_err(gql_err)?
            .map_err(|error| error.to_string())?;
        let storage = storage(ctx)?;
        storage
            .set_user_avatar(user.id, &picture, OffsetDateTime::now_utc())
            .await
            .map_err(gql_err)?;
        storage
            .get_user(user.id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))
    }

    /// Remove the caller's picture, leaving their initials.
    async fn remove_profile_picture(&self, ctx: &Context<'_>) -> async_graphql::Result<User> {
        let mut user = viewer(ctx).await?;
        storage(ctx)?
            .delete_user_avatar(user.id)
            .await
            .map_err(gql_err)?;
        user.avatar_updated_at = None;
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageFormat, RgbImage};

    use super::*;

    fn encoded(picture: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        picture.write_to(&mut out, format).expect("encode");
        out.into_inner()
    }

    #[test]
    fn a_picture_comes_out_a_small_square_with_transparency_on_black() {
        let wide = RgbaImage::from_pixel(1000, 200, Rgba([255, 255, 255, 0]));
        let prepared =
            prepare_picture(&encoded(DynamicImage::ImageRgba8(wide), ImageFormat::Png)).unwrap();
        let out = image::load_from_memory(&prepared)
            .expect("decode")
            .to_rgb8();
        assert_eq!(out.dimensions(), (PICTURE_SIDE, PICTURE_SIDE));
        assert!(out.get_pixel(256, 256).0.iter().all(|&channel| channel < 8));
    }

    #[test]
    fn only_an_iso_file_with_a_heif_brand_goes_to_ffmpeg() {
        let ftyp = |brands: &[u8]| {
            let size = (8 + brands.len()) as u32;
            let mut file = size.to_be_bytes().to_vec();
            file.extend_from_slice(b"ftyp");
            file.extend_from_slice(brands);
            file
        };
        assert!(is_heif(&ftyp(b"avif\0\0\0\0mif1miaf")));
        assert!(is_heif(&ftyp(b"mif1\0\0\0\0heic")));
        assert!(!is_heif(&ftyp(b"isom\0\0\x02\0mp41")), "a video");
        assert!(!is_heif(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_heif(b"ab"));
    }

    #[tokio::test]
    async fn an_avif_comes_through_ffmpeg_as_a_picture_we_can_keep() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.avif");
        let encoded = tokio::process::Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-f", "lavfi"])
            .args(["-i", "color=c=red:s=300x100", "-frames:v", "1"])
            .args([
                "-c:v",
                "libaom-av1",
                "-cpu-used",
                "8",
                "-still-picture",
                "1",
            ])
            .arg(&path)
            .status()
            .await;
        if !encoded.is_ok_and(|status| status.success()) {
            eprintln!("ffmpeg can't write AVIF here; skipping");
            return;
        }
        let avif = std::fs::read(&path).unwrap();
        assert!(is_heif(&avif));
        let png = decode_with_ffmpeg(&avif).await.expect("decoded");
        let prepared = prepare_picture(&png).expect("prepared");
        let out = image::load_from_memory(&prepared).expect("jpeg").to_rgb8();
        assert_eq!(out.dimensions(), (PICTURE_SIDE, PICTURE_SIDE));
        let [red, green, blue] = out.get_pixel(256, 256).0;
        assert!(red > 200 && green < 60 && blue < 60, "{red} {green} {blue}");

        assert!(matches!(
            decode_with_ffmpeg(b"\0\0\0\x10ftypavifnonsense").await,
            Err(PictureError::Unreadable)
        ));
    }

    #[test]
    fn what_isnt_a_picture_or_is_too_big_to_decode_is_refused() {
        assert!(matches!(
            prepare_picture(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            Err(PictureError::Unreadable)
        ));
        let huge = RgbImage::new(MAX_PICTURE_SIDE + 1, 1);
        assert!(matches!(
            prepare_picture(&encoded(DynamicImage::ImageRgb8(huge), ImageFormat::Png)),
            Err(PictureError::Unreadable)
        ));
    }
}
