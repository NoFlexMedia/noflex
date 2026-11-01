// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use noflex_core::{ChannelId, EpgProgramId, LiveTvSourceId};
use noflex_storage::{EpgProgram, LiveTvChannel, Storage, StorageError};
use quick_xml::{Reader, events::Event};
use time::{OffsetDateTime, format_description::FormatItem, macros::format_description};
use tokio::sync::Mutex;
use url::Url;

const PLAYLIST_CAP: usize = 8 * 1024 * 1024;
const XMLTV_CAP: usize = 64 * 1024 * 1024;
const XMLTV_TIMESTAMP: &[FormatItem<'static>] = format_description!(
    "[year][month][day][hour][minute][second] [offset_hour sign:mandatory][offset_minute]"
);

#[derive(Debug, thiserror::Error)]
pub enum LiveTvError {
    #[error("source URL must use http or https")]
    InvalidUrl,
    #[error("source response exceeds {0} bytes")]
    ResponseTooLarge(usize),
    #[error("source request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid UTF-8 in source")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("invalid playlist: {0}")]
    Playlist(String),
    #[error("invalid XMLTV: {0}")]
    Xml(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M3uChannel {
    pub external_id: String,
    pub name: String,
    pub number: Option<String>,
    pub logo_url: Option<String>,
    pub group: Option<String>,
    pub stream_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlTvProgram {
    pub channel_external_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub start_at: OffsetDateTime,
    pub end_at: OffsetDateTime,
}

pub fn parse_m3u(input: &[u8], cap: usize) -> Result<Vec<M3uChannel>, LiveTvError> {
    if input.len() > cap {
        return Err(LiveTvError::ResponseTooLarge(cap));
    }
    let text = std::str::from_utf8(input)?;
    let mut pending: Option<(HashMap<String, String>, String)> = None;
    let mut channels = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (metadata, name) = split_extinf(info)
                .ok_or_else(|| LiveTvError::Playlist("EXTINF has no channel name".into()))?;
            pending = Some((parse_attributes(metadata), name.trim().to_string()));
        } else if !line.starts_with('#') {
            let Some((attrs, fallback_name)) = pending.take() else {
                continue;
            };
            let stream = Url::parse(line)
                .map_err(|_| LiveTvError::Playlist("stream URL is not absolute".into()))?;
            // Same allow-list the playback and recorder ffmpeg inputs
            // enforce, so an imported channel is always playable.
            if !noflex_transcoder::NETWORK_INPUT_SCHEMES.contains(&stream.scheme()) {
                return Err(LiveTvError::Playlist(format!(
                    "stream URL {line:?} must use http or https, not {}",
                    stream.scheme()
                )));
            }
            let name = attrs
                .get("tvg-name")
                .filter(|v| !v.is_empty())
                .cloned()
                .unwrap_or(fallback_name);
            let external_id = attrs
                .get("tvg-id")
                .filter(|v| !v.is_empty())
                .cloned()
                .unwrap_or_else(|| name.clone());
            channels.push(M3uChannel {
                external_id,
                name,
                number: attrs.get("tvg-chno").cloned(),
                logo_url: attrs.get("tvg-logo").cloned(),
                group: attrs.get("group-title").cloned(),
                stream_url: stream.into(),
            });
        }
    }
    if channels.is_empty() {
        return Err(LiveTvError::Playlist(
            "playlist contains no channels".into(),
        ));
    }
    Ok(channels)
}

fn split_extinf(value: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => return Some((&value[..index], &value[index + 1..])),
            _ => {}
        }
    }
    None
}

fn parse_attributes(input: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let key_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b'=' && !bytes[cursor].is_ascii_whitespace()
        {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            cursor += 1;
            continue;
        }
        let key = &input[key_start..cursor];
        cursor += 1;
        let value = if cursor < bytes.len() && bytes[cursor] == b'"' {
            cursor += 1;
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'"' {
                cursor += 1;
            }
            let value = &input[start..cursor];
            cursor = cursor.saturating_add(1);
            value
        } else {
            let start = cursor;
            while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            &input[start..cursor]
        };
        attrs.insert(key.to_ascii_lowercase(), value.to_string());
    }
    attrs
}

pub fn parse_xmltv(input: &[u8], cap: usize) -> Result<Vec<XmlTvProgram>, LiveTvError> {
    if input.len() > cap {
        return Err(LiveTvError::ResponseTooLarge(cap));
    }
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut current: Option<XmlTvProgram> = None;
    let mut field: Option<Vec<u8>> = None;
    let mut out = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| LiveTvError::Xml(e.to_string()))?
        {
            Event::Start(e) if e.name().as_ref() == b"programme" => {
                let mut channel = None;
                let mut start = None;
                let mut stop = None;
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| LiveTvError::Xml(e.to_string()))?;
                    let value = attr
                        .unescape_value()
                        .map_err(|e| LiveTvError::Xml(e.to_string()))?
                        .into_owned();
                    match attr.key.as_ref() {
                        b"channel" => channel = Some(value),
                        b"start" => start = Some(parse_xmltv_time(&value)?),
                        b"stop" => stop = Some(parse_xmltv_time(&value)?),
                        _ => {}
                    }
                }
                let channel =
                    channel.ok_or_else(|| LiveTvError::Xml("programme missing channel".into()))?;
                current = Some(XmlTvProgram {
                    channel_external_id: channel,
                    title: String::new(),
                    subtitle: None,
                    description: None,
                    category: None,
                    start_at: start
                        .ok_or_else(|| LiveTvError::Xml("programme missing start".into()))?,
                    end_at: stop
                        .ok_or_else(|| LiveTvError::Xml("programme missing stop".into()))?,
                });
            }
            Event::Start(e)
                if current.is_some()
                    && matches!(
                        e.name().as_ref(),
                        b"title" | b"sub-title" | b"desc" | b"category"
                    ) =>
            {
                field = Some(e.name().as_ref().to_vec())
            }
            Event::Text(e) if current.is_some() && field.is_some() => {
                let text = e.decode().map_err(|e| LiveTvError::Xml(e.to_string()))?;
                append_xmltv_field(&mut current, field.as_deref(), &text)?;
            }
            Event::CData(e) if current.is_some() && field.is_some() => {
                let text = e.decode().map_err(|e| LiveTvError::Xml(e.to_string()))?;
                append_xmltv_field(&mut current, field.as_deref(), &text)?;
            }
            Event::GeneralRef(e) if current.is_some() && field.is_some() => {
                let text = match e.resolve_char_ref() {
                    Ok(Some(character)) => character.to_string(),
                    Ok(None) => {
                        let entity: &[u8] = &e;
                        match entity {
                            b"amp" => "&".to_string(),
                            b"apos" => "'".to_string(),
                            b"gt" => ">".to_string(),
                            b"lt" => "<".to_string(),
                            b"quot" => "\"".to_string(),
                            _ => {
                                return Err(LiveTvError::Xml(
                                    "unknown XML entity reference".into(),
                                ));
                            }
                        }
                    }
                    Err(error) => return Err(LiveTvError::Xml(error.to_string())),
                };
                append_xmltv_field(&mut current, field.as_deref(), &text)?;
            }
            Event::End(e) if e.name().as_ref() == b"programme" => {
                let p = current
                    .take()
                    .ok_or_else(|| LiveTvError::Xml("unexpected programme close".into()))?;
                if p.title.is_empty() {
                    return Err(LiveTvError::Xml("programme missing title".into()));
                }
                if p.end_at <= p.start_at {
                    return Err(LiveTvError::Xml("programme stop is not after start".into()));
                }
                out.push(p);
                field = None;
            }
            Event::End(_) => field = None,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if current.is_some() {
        return Err(LiveTvError::Xml("unterminated programme".into()));
    }
    Ok(out)
}

fn append_xmltv_field(
    current: &mut Option<XmlTvProgram>,
    field: Option<&[u8]>,
    text: &str,
) -> Result<(), LiveTvError> {
    let program = current
        .as_mut()
        .ok_or_else(|| LiveTvError::Xml("text outside programme".into()))?;
    match field {
        Some(b"title") => program.title.push_str(text),
        Some(b"sub-title") => program.subtitle.get_or_insert_default().push_str(text),
        Some(b"desc") => program.description.get_or_insert_default().push_str(text),
        Some(b"category") => program.category.get_or_insert_default().push_str(text),
        _ => {}
    }
    Ok(())
}

fn parse_xmltv_time(value: &str) -> Result<OffsetDateTime, LiveTvError> {
    OffsetDateTime::parse(value.trim(), XMLTV_TIMESTAMP)
        .map_err(|_| LiveTvError::Xml(format!("invalid XMLTV timestamp {value}")))
}

pub struct LiveTvIngest {
    storage: Arc<dyn Storage>,
    client: reqwest::Client,
    source_locks: Mutex<HashMap<LiveTvSourceId, Arc<Mutex<()>>>>,
}

impl LiveTvIngest {
    pub fn new(storage: Arc<dyn Storage>) -> Result<Self, LiveTvError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            storage,
            client,
            source_locks: Mutex::new(HashMap::new()),
        })
    }

    pub async fn refresh(
        &self,
        source_id: LiveTvSourceId,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<(), LiveTvError> {
        let source_lock = {
            let mut locks = self.source_locks.lock().await;
            locks
                .entry(source_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = source_lock.lock().await;
        let source = self
            .storage
            .get_live_tv_source(source_id)
            .await?
            .ok_or(StorageError::NotFound)?;
        let attempted_at = OffsetDateTime::now_utc();
        let result = self.refresh_inner(&source, start, end, attempted_at).await;
        if let Err(error) = &result {
            self.storage
                .record_live_tv_refresh_error(source_id, &error.to_string(), attempted_at)
                .await?;
        }
        result
    }

    async fn refresh_inner(
        &self,
        source: &noflex_storage::LiveTvSource,
        start: OffsetDateTime,
        end: OffsetDateTime,
        refreshed_at: OffsetDateTime,
    ) -> Result<(), LiveTvError> {
        let playlist = fetch_capped(&self.client, &source.playlist_url, PLAYLIST_CAP).await?;
        let parsed = parse_m3u(&playlist, PLAYLIST_CAP)?;
        let mut channel_ids = HashMap::new();
        let channels = parsed
            .into_iter()
            .map(|item| {
                let id = ChannelId::new();
                channel_ids.insert(item.external_id.clone(), id);
                LiveTvChannel {
                    id,
                    source_id: source.id,
                    external_id: item.external_id,
                    name: item.name,
                    number: item.number,
                    logo_url: item.logo_url,
                    group: item.group,
                    enabled: true,
                    stream_url: item.stream_url,
                }
            })
            .collect::<Vec<_>>();
        let programs = if let Some(url) = source.epg_url.as_deref() {
            let xml = fetch_capped(&self.client, url, XMLTV_CAP).await?;
            parse_xmltv(&xml, XMLTV_CAP)?
                .into_iter()
                .filter_map(|p| {
                    let channel_id = *channel_ids.get(&p.channel_external_id)?;
                    Some(EpgProgram {
                        id: EpgProgramId::new(),
                        channel_id,
                        external_id: format!(
                            "{}:{}",
                            p.channel_external_id,
                            p.start_at.unix_timestamp()
                        ),
                        title: p.title,
                        subtitle: p.subtitle,
                        description: p.description,
                        category: p.category,
                        start_at: p.start_at,
                        end_at: p.end_at,
                    })
                })
                .filter(|p| p.start_at < end && p.end_at > start)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.storage
            .apply_live_tv_refresh(source.id, &channels, &programs, refreshed_at)
            .await?;
        Ok(())
    }
}

pub(crate) async fn fetch_capped(
    client: &reqwest::Client,
    url: &str,
    cap: usize,
) -> Result<Vec<u8>, LiveTvError> {
    let parsed = Url::parse(url).map_err(|_| LiveTvError::InvalidUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(LiveTvError::InvalidUrl);
    }
    let response = client.get(parsed).send().await?.error_for_status()?;
    if response.content_length().is_some_and(|v| v > cap as u64) {
        return Err(LiveTvError::ResponseTooLarge(cap));
    }
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if out.len().saturating_add(chunk.len()) > cap {
            return Err(LiveTvError::ResponseTooLarge(cap));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn local_response(body: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local source");
        let address = listener.local_addr().expect("local source address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept source request");
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .expect("write source headers");
            socket.write_all(body).await.expect("write source body");
        });
        format!("http://{address}/source")
    }
    #[test]
    fn parses_m3u_attributes() {
        let rows=parse_m3u(b"#EXTM3U\n#EXTINF:-1 tvg-id=\"news\" tvg-name=\"News HD\" tvg-logo=\"https://img/logo.png\" tvg-chno=\"7\" group-title=\"News\",News\nhttps://tv/stream\n",1024).unwrap();
        assert_eq!(rows[0].external_id, "news");
        assert_eq!(rows[0].number.as_deref(), Some("7"));
    }
    #[test]
    fn rejects_non_http_stream_schemes() {
        for url in [
            "file:///etc/passwd",
            "rtmp://tv/live",
            "udp://239.0.0.1:1234",
        ] {
            let playlist = format!("#EXTINF:-1,News\n{url}\n");
            let err = parse_m3u(playlist.as_bytes(), 1024).expect_err(url);
            assert!(err.to_string().contains("http or https"), "{err}");
        }
    }
    #[test]
    fn rejects_relative_and_caps() {
        assert!(parse_m3u(b"#EXTINF:-1,News\n/live\n", 1024).is_err());
        assert!(matches!(
            parse_m3u(b"abcd", 3),
            Err(LiveTvError::ResponseTooLarge(3))
        ));
    }
    #[test]
    fn parses_timezone_aware_xmltv() {
        let rows=parse_xmltv(br#"<tv><programme channel="news" start="20250101120000 +0530" stop="20250101123000 +0530"><title>Lunch</title><sub-title>Headlines</sub-title><category>News</category></programme></tv>"#,4096).unwrap();
        assert_eq!(rows[0].start_at.offset().whole_hours(), 5);
        assert_eq!(rows[0].start_at.offset().minutes_past_hour(), 30);
    }
    #[tokio::test]
    async fn fetches_private_host_and_enforces_body_cap() {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("HTTP client");
        let url = local_response(b"private tuner").await;
        assert_eq!(
            fetch_capped(&client, &url, 64).await.expect("LAN fetch"),
            b"private tuner"
        );
        let url = local_response(b"too large").await;
        assert!(matches!(
            fetch_capped(&client, &url, 3).await,
            Err(LiveTvError::ResponseTooLarge(3))
        ));
    }

    #[test]
    fn rejects_invalid_xmltv_time() {
        assert!(parse_xmltv(br#"<tv><programme channel="x" start="bad" stop="20250101123000 +0000"><title>X</title></programme></tv>"#,4096).is_err());
    }
}
