// SPDX-License-Identifier: AGPL-3.0-only
//
// The single file-serving path for every byte route: direct play,
// export-job downloads, subtitle sidecars, HLS artifacts, and
// images all delegate here. Bodies stream via `ReaderStream`;
// nothing is buffered whole.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

/// How a request's Range header resolves against the actual file
/// size, per RFC 7233.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeParse {
    /// No header, or one we can't honor as a single byte range
    /// (bad syntax, multi-range, zero/oversized suffix): ignore
    /// the header and serve the full body with a 200.
    Full,
    /// A well-formed, in-bounds range; `end` is inclusive and
    /// clamped to the file size.
    Satisfiable { start: u64, end: u64 },
    /// Well-formed but out of bounds for this file (start past
    /// the end, or a reversed range): the answer is a 416.
    Unsatisfiable,
}

/// Parse the `bytes=START-END`, `bytes=START-`, and `bytes=-N`
/// forms against `size`. Multi-range requests get `Full` — no
/// client we serve asks for multipart/byteranges.
pub fn parse_range(header: Option<&str>, size: u64) -> RangeParse {
    let Some(h) = header.and_then(|h| h.strip_prefix("bytes=")) else {
        return RangeParse::Full;
    };
    let h = h.trim();
    if h.contains(',') {
        return RangeParse::Full;
    }
    let Some((s, e)) = h.split_once('-') else {
        return RangeParse::Full;
    };
    if s.is_empty() {
        // Suffix form: the last N bytes. Zero length and a suffix
        // longer than the file aren't satisfiable as a range, so
        // the header is ignored rather than clamped.
        let Ok(suffix) = e.parse::<u64>() else {
            return RangeParse::Full;
        };
        if suffix == 0 || suffix > size {
            return RangeParse::Full;
        }
        RangeParse::Satisfiable {
            start: size - suffix,
            end: size - 1,
        }
    } else {
        let Ok(start) = s.parse::<u64>() else {
            return RangeParse::Full;
        };
        if start >= size {
            return RangeParse::Unsatisfiable;
        }
        if e.is_empty() {
            return RangeParse::Satisfiable {
                start,
                end: size - 1,
            };
        }
        let Ok(end) = e.parse::<u64>() else {
            return RangeParse::Full;
        };
        if start > end {
            return RangeParse::Unsatisfiable;
        }
        RangeParse::Satisfiable {
            start,
            end: end.min(size - 1),
        }
    }
}

/// Extension → MIME for everything the byte routes serve. This is
/// the one table; routes must not carry private guesses.
pub fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("ts" | "m2ts") => "video/mp2t",
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("mp3") => "audio/mpeg",
        Some("m4a" | "aac") => "audio/aac",
        Some("flac") => "audio/flac",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("srt") => "application/x-subrip",
        Some("vtt") => "text/vtt",
        Some("ass" | "ssa") => "text/x-ssa",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfinedOpenError {
    #[error("filesystem root is unavailable: {0}")]
    RootUnavailable(#[source] io::Error),
    #[error("file is unavailable: {0}")]
    Unavailable(#[source] io::Error),
    #[error("path is outside the configured root")]
    OutsideRoot,
    #[error("path is not a regular file")]
    NotRegular,
}

/// A canonical filesystem root used as an authority to open files beneath it.
///
/// The returned [`OpenedFile`] is the only supported input to the byte-serving
/// boundary, so validation and serving cannot accidentally become separate
/// path operations. Unix retains an open directory descriptor and traverses
/// from it with `openat` and `O_NOFOLLOW`, closing symlink-swap windows.
#[derive(Debug, Clone)]
pub struct RootCapability {
    root: PathBuf,
    #[cfg(unix)]
    root_dir: Arc<std::fs::File>,
}

impl RootCapability {
    pub async fn new(root: impl AsRef<Path>) -> Result<Self, ConfinedOpenError> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(ConfinedOpenError::RootUnavailable)?;
        let metadata = tokio::fs::metadata(&root)
            .await
            .map_err(ConfinedOpenError::RootUnavailable)?;
        if !metadata.is_dir() {
            return Err(ConfinedOpenError::RootUnavailable(io::Error::new(
                io::ErrorKind::NotADirectory,
                "configured root is not a directory",
            )));
        }
        #[cfg(unix)]
        let root_dir = {
            use std::os::unix::fs::OpenOptionsExt as _;

            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&root)
                .map(Arc::new)
                .map_err(ConfinedOpenError::RootUnavailable)?
        };
        Ok(Self {
            root,
            #[cfg(unix)]
            root_dir,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub async fn open(&self, path: impl AsRef<Path>) -> Result<OpenedFile, ConfinedOpenError> {
        let canonical = tokio::fs::canonicalize(path)
            .await
            .map_err(ConfinedOpenError::Unavailable)?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| ConfinedOpenError::OutsideRoot)?
            .to_path_buf();

        #[cfg(unix)]
        return self.open_unix(&relative, canonical).await;

        #[cfg(not(unix))]
        self.open_non_unix(canonical).await
    }

    #[cfg(unix)]
    async fn open_unix(
        &self,
        relative: &Path,
        canonical: PathBuf,
    ) -> Result<OpenedFile, ConfinedOpenError> {
        use std::{
            ffi::CString,
            os::{
                fd::{AsRawFd as _, FromRawFd as _},
                unix::ffi::OsStrExt as _,
            },
        };

        let components = relative
            .components()
            .map(|component| match component {
                std::path::Component::Normal(component) => {
                    CString::new(component.as_bytes()).map_err(|_| ConfinedOpenError::OutsideRoot)
                }
                _ => Err(ConfinedOpenError::OutsideRoot),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut parents = components;
        let Some(last) = parents.pop() else {
            return Err(ConfinedOpenError::NotRegular);
        };

        let root_dir = self.root_dir.clone();
        // Blocking syscalls, so off the async workers. The final open is
        // O_NONBLOCK so a FIFO (or device) planted under a root fails the
        // regular-file check instead of parking the thread on open().
        let (file, metadata) = tokio::task::spawn_blocking(move || {
            let mut directory = root_dir
                .try_clone()
                .map_err(ConfinedOpenError::Unavailable)?;
            for component in parents {
                let fd = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        component.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(ConfinedOpenError::Unavailable(io::Error::last_os_error()));
                }
                directory = unsafe { std::fs::File::from_raw_fd(fd) };
            }
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    last.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err(ConfinedOpenError::Unavailable(io::Error::last_os_error()));
            }
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            let metadata = file.metadata().map_err(ConfinedOpenError::Unavailable)?;
            if !metadata.is_file() {
                return Err(ConfinedOpenError::NotRegular);
            }
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0
            {
                return Err(ConfinedOpenError::Unavailable(io::Error::last_os_error()));
            }
            Ok((file, metadata))
        })
        .await
        .map_err(|error| ConfinedOpenError::Unavailable(io::Error::other(error)))??;
        Ok(OpenedFile {
            file: tokio::fs::File::from_std(file),
            path: canonical,
            metadata,
        })
    }

    #[cfg(not(unix))]
    async fn open_non_unix(&self, canonical: PathBuf) -> Result<OpenedFile, ConfinedOpenError> {
        let file = tokio::fs::File::open(&canonical)
            .await
            .map_err(ConfinedOpenError::Unavailable)?;
        let metadata = file
            .metadata()
            .await
            .map_err(ConfinedOpenError::Unavailable)?;
        if !metadata.is_file() {
            return Err(ConfinedOpenError::NotRegular);
        }
        Ok(OpenedFile {
            file,
            path: canonical,
            metadata,
        })
    }
}

#[derive(Debug)]
pub struct OpenedFile {
    file: tokio::fs::File,
    path: PathBuf,
    metadata: std::fs::Metadata,
}

impl OpenedFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn metadata(&self) -> &std::fs::Metadata {
        &self.metadata
    }

    pub fn into_file(self) -> tokio::fs::File {
        self.file
    }
}

/// Open `path` under one of the configured roots.
pub async fn open_from_roots(
    roots: &[String],
    path: impl AsRef<Path>,
) -> Result<OpenedFile, ConfinedOpenError> {
    let path = path.as_ref();
    let mut unavailable = None;
    for root in roots {
        let capability = match RootCapability::new(root).await {
            Ok(capability) => capability,
            Err(error) => {
                unavailable = Some(error);
                continue;
            }
        };
        match capability.open(path).await {
            Ok(file) => return Ok(file),
            Err(ConfinedOpenError::OutsideRoot) => {}
            Err(error) => return Err(error),
        }
    }
    Err(unavailable.unwrap_or(ConfinedOpenError::OutsideRoot))
}

/// Per-route extras layered onto the shared response.
#[derive(Debug, Clone, Default)]
pub struct ServeOpts {
    /// e.g. `attachment; filename="movie.mp4"` for downloads.
    pub content_disposition: Option<String>,
    pub cache_control: Option<&'static str>,
}

/// Serve an already-open file, honoring `range_header`: 200 with the full
/// body, 206 with the requested slice, or 416 when the range is unsatisfiable.
pub async fn serve_file(
    opened: OpenedFile,
    range_header: Option<&str>,
    opts: ServeOpts,
) -> Response {
    let size = opened.metadata.len();
    let path = opened.path;
    let mut file = opened.file;

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_for(&path)),
    );
    headers.insert(
        axum::http::header::ACCEPT_RANGES,
        HeaderValue::from_static("bytes"),
    );
    if let Some(disposition) = opts.content_disposition {
        headers.insert(
            axum::http::header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition)
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        );
    }
    if let Some(cache_control) = opts.cache_control {
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static(cache_control),
        );
    }

    match parse_range(range_header, size) {
        RangeParse::Full => {
            headers.insert(
                axum::http::header::CONTENT_LENGTH,
                HeaderValue::from_str(&size.to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("0")),
            );
            (
                StatusCode::OK,
                headers,
                Body::from_stream(ReaderStream::new(file)),
            )
                .into_response()
        }
        RangeParse::Unsatisfiable => {
            headers.insert(
                axum::http::header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{size}"))
                    .unwrap_or_else(|_| HeaderValue::from_static("bytes */*")),
            );
            (StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response()
        }
        RangeParse::Satisfiable { start, end } => {
            if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
                tracing::error!(error = %e, path = %path.display(), "seek failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "seek failed").into_response();
            }
            let length = end - start + 1;
            headers.insert(
                axum::http::header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))
                    .unwrap_or_else(|_| HeaderValue::from_static("bytes")),
            );
            headers.insert(
                axum::http::header::CONTENT_LENGTH,
                HeaderValue::from_str(&length.to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("0")),
            );
            (
                StatusCode::PARTIAL_CONTENT,
                headers,
                Body::from_stream(ReaderStream::new(file.take(length))),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("noflex-fs-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(contents).unwrap();
        p
    }

    #[test]
    fn range_start_end() {
        assert_eq!(
            parse_range(Some("bytes=10-19"), 100),
            RangeParse::Satisfiable { start: 10, end: 19 }
        );
    }

    #[test]
    fn range_open_ended() {
        assert_eq!(
            parse_range(Some("bytes=10-"), 100),
            RangeParse::Satisfiable { start: 10, end: 99 }
        );
    }

    #[test]
    fn range_suffix() {
        assert_eq!(
            parse_range(Some("bytes=-5"), 100),
            RangeParse::Satisfiable { start: 95, end: 99 }
        );
    }

    #[test]
    fn range_end_clamps_to_size() {
        assert_eq!(
            parse_range(Some("bytes=0-200"), 100),
            RangeParse::Satisfiable { start: 0, end: 99 }
        );
    }

    #[test]
    fn range_out_of_bounds_is_unsatisfiable() {
        assert_eq!(
            parse_range(Some("bytes=200-"), 100),
            RangeParse::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=100-200"), 100),
            RangeParse::Unsatisfiable
        );
    }

    #[test]
    fn range_reversed_is_unsatisfiable() {
        assert_eq!(
            parse_range(Some("bytes=20-10"), 100),
            RangeParse::Unsatisfiable
        );
    }

    #[test]
    fn malformed_range_falls_back_to_full() {
        assert_eq!(parse_range(None, 100), RangeParse::Full);
        assert_eq!(parse_range(Some(""), 100), RangeParse::Full);
        assert_eq!(parse_range(Some("items=0-10"), 100), RangeParse::Full);
        assert_eq!(parse_range(Some("bytes=abc"), 100), RangeParse::Full);
        assert_eq!(parse_range(Some("bytes=0-10,20-30"), 100), RangeParse::Full);
        assert_eq!(parse_range(Some("bytes=-0"), 100), RangeParse::Full);
        assert_eq!(parse_range(Some("bytes=-101"), 100), RangeParse::Full);
        assert_eq!(parse_range(Some("bytes=10-x"), 100), RangeParse::Full);
    }

    #[test]
    fn content_type_guesses_common_video() {
        assert_eq!(content_type_for(Path::new("a.mp4")), "video/mp4");
        assert_eq!(content_type_for(Path::new("a.m4v")), "video/mp4");
        assert_eq!(content_type_for(Path::new("a.mkv")), "video/x-matroska");
        assert_eq!(content_type_for(Path::new("a.webm")), "video/webm");
    }

    #[test]
    fn content_type_guesses_audio() {
        assert_eq!(content_type_for(Path::new("a.mp3")), "audio/mpeg");
        assert_eq!(content_type_for(Path::new("a.flac")), "audio/flac");
    }

    #[test]
    fn content_type_guesses_subtitles() {
        assert_eq!(content_type_for(Path::new("a.srt")), "application/x-subrip");
        assert_eq!(content_type_for(Path::new("a.vtt")), "text/vtt");
        assert_eq!(content_type_for(Path::new("a.ass")), "text/x-ssa");
        assert_eq!(content_type_for(Path::new("a.ssa")), "text/x-ssa");
    }

    #[test]
    fn content_type_guesses_images_and_hls() {
        assert_eq!(content_type_for(Path::new("a.gif")), "image/gif");
        assert_eq!(content_type_for(Path::new("a.ts")), "video/mp2t");
        assert_eq!(
            content_type_for(Path::new("index.m3u8")),
            "application/vnd.apple.mpegurl"
        );
    }

    #[test]
    fn content_type_falls_back_to_octet_stream() {
        assert_eq!(
            content_type_for(Path::new("a.xyz")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type_for(Path::new("README")),
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn serve_full_file() {
        let p = temp_file("file.mp4", b"hello world");
        let root = RootCapability::new(p.parent().unwrap()).await.unwrap();
        let resp = serve_file(root.open(&p).await.unwrap(), None, ServeOpts::default()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok()),
            Some("11")
        );
        assert_eq!(
            resp.headers()
                .get(axum::http::header::ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok()),
            Some("bytes")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"hello world");
    }

    #[tokio::test]
    async fn serve_range_subset() {
        let p = temp_file("file.mp4", b"0123456789ABCDEF");
        let root = RootCapability::new(p.parent().unwrap()).await.unwrap();
        let resp = serve_file(
            root.open(&p).await.unwrap(),
            Some("bytes=4-7"),
            ServeOpts::default(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok()),
            Some("bytes 4-7/16")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"4567");
    }

    #[tokio::test]
    async fn serve_unsatisfiable_range_is_416() {
        let p = temp_file("file.mp4", b"0123456789");
        let root = RootCapability::new(p.parent().unwrap()).await.unwrap();
        let resp = serve_file(
            root.open(&p).await.unwrap(),
            Some("bytes=50-"),
            ServeOpts::default(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok()),
            Some("bytes */10")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn serve_opts_set_extra_headers() {
        let p = temp_file("movie.mp4", b"data");
        let root = RootCapability::new(p.parent().unwrap()).await.unwrap();
        let resp = serve_file(
            root.open(&p).await.unwrap(),
            None,
            ServeOpts {
                content_disposition: Some("attachment; filename=\"movie.mp4\"".to_string()),
                cache_control: Some("public, max-age=86400"),
            },
        )
        .await;
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok()),
            Some("attachment; filename=\"movie.mp4\"")
        );
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("public, max-age=86400")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fifo_under_root_is_rejected_without_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let fifo = temp.path().join("trap.mkv");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let root = RootCapability::new(temp.path()).await.unwrap();
        let opened = tokio::time::timeout(std::time::Duration::from_secs(5), root.open(&fifo))
            .await
            .expect("open must not block on a FIFO");
        assert!(matches!(opened, Err(ConfinedOpenError::NotRegular)));
    }

    #[tokio::test]
    async fn root_rejects_outside_and_sibling_prefix_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root_dir = temp.path().join("media");
        let sibling = temp.path().join("media-private");
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let outside = temp.path().join("outside.mp4");
        let sibling_file = sibling.join("movie.mp4");
        std::fs::write(&outside, b"outside").unwrap();
        std::fs::write(&sibling_file, b"sibling").unwrap();
        let root = RootCapability::new(&root_dir).await.unwrap();

        assert!(matches!(
            root.open(&outside).await,
            Err(ConfinedOpenError::OutsideRoot)
        ));
        assert!(matches!(
            root.open(&sibling_file).await,
            Err(ConfinedOpenError::OutsideRoot)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn root_rejects_final_and_intermediate_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root_dir = temp.path().join("media");
        let outside_dir = temp.path().join("outside");
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("movie.mp4");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, root_dir.join("final.mp4")).unwrap();
        symlink(&outside_dir, root_dir.join("nested")).unwrap();
        let root = RootCapability::new(&root_dir).await.unwrap();

        assert!(matches!(
            root.open(root_dir.join("final.mp4")).await,
            Err(ConfinedOpenError::OutsideRoot)
        ));
        assert!(matches!(
            root.open(root_dir.join("nested/movie.mp4")).await,
            Err(ConfinedOpenError::OutsideRoot)
        ));
    }

    #[tokio::test]
    async fn root_rejects_non_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        let root = RootCapability::new(temp.path()).await.unwrap();
        assert!(matches!(
            root.open(directory).await,
            Err(ConfinedOpenError::NotRegular)
        ));
    }
}
