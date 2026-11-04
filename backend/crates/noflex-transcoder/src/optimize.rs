// SPDX-License-Identifier: AGPL-3.0-only
//
// Which optimized versions of a source to prepare ahead of time for a set of
// devices, and the ffmpeg arguments that make one.

use std::{collections::BTreeMap, path::Path};

use noflex_core::{AudioCodec, DeviceId, HdrTransfer, VideoCodec};
use serde::{Deserialize, Serialize};

use crate::{
    decision::{
        AudioPlan, ClientCapabilities, DecideOptions, HlsSegmentContainer, PlaybackRoute,
        TranscodePlan, VideoPlan, VideoRange, decide,
    },
    encoder::Encoder,
    ffmpeg_argv::{
        LadderEntry, TranscodeInput, TranscodeRequest, audio_encoder_flag, push_video,
        scaled_bitrate_kbps,
    },
    hls::{HlsVariant, HlsVideoRange, audio_bitrate_bps, audio_codec_string, video_codec_string},
    package::{AudioRendition, PACKAGE_MOVFLAGS, PackageIndex, TrackKind, master_playlist},
    probe::{AudioStreamProbe, ProbeReport, map_audio_codec},
};

/// Newer codecs reach the same quality in fewer bits than the H.264 rates
/// ladders are usually written for.
const EFFICIENT_CODEC_BITRATE_PERCENT: u32 = 70;

/// A device a version could be prepared for.
#[derive(Debug, Clone)]
pub struct DeviceTarget {
    pub id: DeviceId,
    pub capabilities: ClientCapabilities,
}

/// How to treat one of the source's audio streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTarget {
    /// Index among the source's audio streams.
    pub stream: usize,
    pub plan: AudioPlan,
}

/// One version to prepare, and the devices it serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizeTarget {
    pub video: VideoPlan,
    /// Only meaningful when `video` encodes.
    pub bitrate_kbps: u32,
    pub audio: Vec<AudioTarget>,
    pub devices: Vec<DeviceId>,
}

impl OptimizeTarget {
    /// Identifies what this version contains, so a planned version can be
    /// matched to one already prepared from the same source.
    pub fn key(&self) -> String {
        let video = match self.video {
            VideoPlan::None => "none".to_string(),
            VideoPlan::Copy { codec, .. } => format!("copy-{codec:?}"),
            VideoPlan::Encode {
                codec,
                height,
                tonemap,
                profile,
                ..
            } => format!(
                "{codec:?}-{height}p-{}k-{profile:?}{}",
                self.bitrate_kbps,
                if tonemap { "-tonemap" } else { "" }
            ),
        };
        let audio = self
            .audio
            .iter()
            .map(|track| match track.plan {
                AudioPlan::None => "none".to_string(),
                AudioPlan::Copy { .. } => "copy".to_string(),
                AudioPlan::Encode { codec, channels } => format!("{codec:?}{channels}"),
            })
            .collect::<Vec<_>>()
            .join("+");
        format!("{video}/{audio}").to_lowercase()
    }

    /// The video bitrate this version will carry, for estimates. A copy uses
    /// the source's overall rate.
    pub fn estimated_bitrate_bps(&self, probe: &ProbeReport) -> u64 {
        let video = match self.video {
            VideoPlan::Encode { .. } => u64::from(self.bitrate_kbps) * 1000,
            _ => probe.format_bit_rate.unwrap_or(0),
        };
        let audio = self
            .audio
            .iter()
            .map(|track| match track.plan {
                AudioPlan::Encode { codec, channels } => audio_bitrate_bps(codec, channels),
                _ => 192_000,
            })
            .sum::<u64>();
        video + audio
    }
}

/// A device's preferred video and what it would present.
struct Want {
    device: usize,
    video: VideoPlan,
}

/// The versions worth preparing of `probe` for `devices`. Devices that can
/// direct-play the source need nothing, so a source they all play yields an
/// empty list.
///
/// Each remaining device starts from the best video it supports, then devices
/// whose targets match in height and dynamic range share one version when one
/// of those targets plays on them all. A 4K HDR TV and a 1080p SDR phone stay
/// separate: merging them would cost real quality. `video` re-encodes follow
/// `opts.encode_video` preference order, and every audio stream is kept.
///
/// `ladder` sets encode bitrates; a device's `max_bitrate_bps` can only lower
/// them.
pub fn plan_versions(
    probe: &ProbeReport,
    devices: &[DeviceTarget],
    opts: &DecideOptions,
    ladder: &[LadderEntry],
) -> Vec<OptimizeTarget> {
    let source_height = probe.video.as_ref().map_or(0, |video| video.height);
    // Grouped by (height, is HDR); the map keeps the plan's order stable.
    let mut groups: BTreeMap<(u32, bool), Vec<Want>> = BTreeMap::new();
    for (device, target) in devices.iter().enumerate() {
        let capabilities = fragmented_only(&target.capabilities);
        let decision = decide(probe, &capabilities, opts);
        let PlaybackRoute::Transcoding { plan, .. } = decision.route else {
            continue;
        };
        let group = match plan.video {
            VideoPlan::None => continue,
            VideoPlan::Copy { range, .. } => (source_height, range != VideoRange::Sdr),
            VideoPlan::Encode { height, .. } => (height, false),
        };
        groups.entry(group).or_default().push(Want {
            device,
            video: plan.video,
        });
    }

    let mut targets = Vec::new();
    // Tallest first, so the most demanding version is the first one prepared.
    for (_, mut wants) in groups.into_iter().rev() {
        while !wants.is_empty() {
            let chosen = best_shared_video(&wants, devices, opts);
            let (served, rest) = wants.into_iter().partition::<Vec<_>, _>(|want| {
                plays(&chosen, want, &devices[want.device].capabilities)
            });
            wants = rest;
            let capabilities = served
                .iter()
                .map(|want| &devices[want.device].capabilities)
                .collect::<Vec<_>>();
            targets.push(OptimizeTarget {
                video: chosen,
                bitrate_kbps: bitrate_kbps(chosen, &capabilities, ladder),
                audio: audio_targets(&probe.audio, &capabilities),
                devices: served.iter().map(|want| devices[want.device].id).collect(),
            });
        }
    }
    targets
}

/// Whether a client with `capabilities` can play a finished version whose
/// contents are `probe`, with nothing re-encoded.
pub fn version_plays(probe: &ProbeReport, capabilities: &ClientCapabilities) -> bool {
    let decision = decide(
        probe,
        &fragmented_only(capabilities),
        &DecideOptions::default(),
    );
    match decision.route {
        PlaybackRoute::Direct => true,
        PlaybackRoute::Transcoding { plan, .. } => {
            matches!(plan.video, VideoPlan::Copy { .. })
                && matches!(plan.audio, AudioPlan::Copy { .. } | AudioPlan::None)
        }
    }
}

/// The master playlist of a finished version, linking `video.m3u8` and one
/// `audio_{track id}.m3u8` per audio track. `probe` describes the finished file;
/// its audio streams line up with `index`'s audio tracks in order.
pub fn version_master_playlist(index: &PackageIndex, probe: &ProbeReport) -> String {
    let mut codecs = Vec::new();
    let video = probe.video.as_ref();
    if let Some(string) = video.and_then(|video| {
        video_codec_string(video.codec, video.profile, video.level, video.bit_depth)
    }) {
        codecs.push(string);
    }
    let audio_tracks = index
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio);
    let mut renditions = Vec::new();
    for (position, (track, stream)) in audio_tracks.zip(&probe.audio).enumerate() {
        if let Some(string) = map_audio_codec(&stream.codec)
            .and_then(|codec| audio_codec_string(codec, stream.profile.as_deref()))
            && !codecs.iter().any(|known| known == string)
        {
            codecs.push(string.to_string());
        }
        renditions.push(AudioRendition {
            name: stream
                .title
                .clone()
                .or_else(|| stream.language.clone())
                .unwrap_or_else(|| format!("Track {}", position + 1)),
            language: stream.language.clone(),
            default: position == 0,
            uri: format!("audio_{}.m3u8", track.track_id),
        });
    }
    let variant = HlsVariant {
        bandwidth_bps: index
            .size_bytes
            .saturating_mul(8000)
            .checked_div(index.duration_ms)
            .unwrap_or(1),
        resolution: video.map(|video| (video.width, video.height)),
        codecs: codecs.join(","),
        supplemental_codecs: None,
        video_range: video.and_then(|video| match video.transfer {
            Some(HdrTransfer::Pq) => Some(HlsVideoRange::Pq),
            Some(HdrTransfer::Hlg) => Some(HlsVideoRange::Hlg),
            _ => None,
        }),
        path: "video.m3u8".to_string(),
    };
    master_playlist(&variant, &renditions)
}

/// `capabilities` narrowed to what an optimized version can be: fragmented
/// MP4, so never a TS-only plan.
fn fragmented_only(capabilities: &ClientCapabilities) -> ClientCapabilities {
    let mut capabilities = capabilities.clone();
    capabilities
        .hls
        .retain(|profile| profile.segment_container == HlsSegmentContainer::Fmp4);
    capabilities
}

/// Among what `wants` asked for, the video the most of them can play. Ties go
/// to a stream copy, which loses nothing, then to the server's preferred codec.
fn best_shared_video(wants: &[Want], devices: &[DeviceTarget], opts: &DecideOptions) -> VideoPlan {
    let rank = |video: &VideoPlan| match video {
        VideoPlan::Copy { .. } | VideoPlan::None => 0,
        VideoPlan::Encode { codec, .. } => {
            1 + opts
                .encode_video
                .iter()
                .position(|preferred| preferred == codec)
                .unwrap_or(opts.encode_video.len())
        }
    };
    let mut best = wants[0].video;
    let mut best_score = (0, usize::MAX);
    for want in wants {
        let coverage = wants
            .iter()
            .filter(|other| plays(&want.video, other, &devices[other.device].capabilities))
            .count();
        let score = (coverage, rank(&want.video));
        if score.0 > best_score.0 || (score.0 == best_score.0 && score.1 < best_score.1) {
            best = want.video;
            best_score = score;
        }
    }
    best
}

/// Whether a device that wanted `want` can play `video`.
fn plays(video: &VideoPlan, want: &Want, capabilities: &ClientCapabilities) -> bool {
    if *video == want.video {
        return true;
    }
    let VideoPlan::Encode {
        codec,
        profile,
        width,
        height,
        ..
    } = *video
    else {
        // A device that needed an encode couldn't copy the source; one that
        // copies a different stream isn't this device.
        return false;
    };
    let carried = capabilities.hls.iter().any(|hls| {
        hls.segment_container == HlsSegmentContainer::Fmp4 && hls.video_codecs.contains(&codec)
    });
    carried
        && capabilities.max_height.is_none_or(|max| height <= max)
        && capabilities.video_codecs.iter().any(|support| {
            support.codec == codec
                && (support.profiles.is_empty() || support.profiles.contains(&profile))
                && support.fits(width, height)
        })
}

fn bitrate_kbps(
    video: VideoPlan,
    capabilities: &[&ClientCapabilities],
    ladder: &[LadderEntry],
) -> u32 {
    let VideoPlan::Encode {
        codec,
        width,
        height,
        ..
    } = video
    else {
        return 0;
    };
    let pixels = u64::from(width) * u64::from(height);
    let mut kbps = LadderEntry::pick(ladder, codec, pixels, None).map_or(8_000, |rung| {
        let scaled = scaled_bitrate_kbps(rung, pixels);
        if matches!(codec, VideoCodec::H265 | VideoCodec::Av1)
            && rung.video_codec == VideoCodec::H264
        {
            scaled * EFFICIENT_CODEC_BITRATE_PERCENT / 100
        } else {
            scaled
        }
    });
    for limit in capabilities.iter().filter_map(|caps| caps.max_bitrate_bps) {
        kbps = kbps.min(u32::try_from(limit / 1000).unwrap_or(u32::MAX));
    }
    kbps
}

/// Every source audio stream: copied when all of `capabilities` decode it in
/// fragmented MP4, otherwise AAC stereo, which every client supports.
fn audio_targets(
    streams: &[AudioStreamProbe],
    capabilities: &[&ClientCapabilities],
) -> Vec<AudioTarget> {
    streams
        .iter()
        .enumerate()
        .map(|(stream, probe)| {
            let copyable = map_audio_codec(&probe.codec).filter(|codec| {
                capabilities
                    .iter()
                    .all(|caps| decodes_audio(caps, *codec, probe.channels))
            });
            AudioTarget {
                stream,
                plan: match copyable {
                    Some(codec) => AudioPlan::Copy { codec },
                    None => AudioPlan::Encode {
                        codec: AudioCodec::Aac,
                        channels: probe.channels.clamp(1, 2),
                    },
                },
            }
        })
        .collect()
}

fn decodes_audio(capabilities: &ClientCapabilities, codec: AudioCodec, channels: u8) -> bool {
    let carried = capabilities.hls.iter().any(|hls| {
        hls.segment_container == HlsSegmentContainer::Fmp4 && hls.audio_codecs.contains(&codec)
    });
    carried
        && capabilities.audio_codecs.iter().any(|support| {
            support.codec == codec && support.max_channels.is_none_or(|max| channels <= max)
        })
}

/// The ffmpeg arguments writing `target` of `input` to `output`, `argv[0] ==
/// "ffmpeg"`, in the layout [`crate::package::finalize`] expects. Progress goes
/// to stdout as `key=value` blocks.
pub fn build_optimize_argv(
    input: &Path,
    target: &OptimizeTarget,
    encoder: Encoder,
    vaapi_device: Option<&Path>,
    output: &Path,
) -> Vec<String> {
    let mut argv: Vec<String> = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
    ]
    .map(String::from)
    .to_vec();
    if matches!(target.video, VideoPlan::Encode { .. }) {
        argv.extend(encoder.extra_args(vaapi_device));
    }
    let source = TranscodeInput::File(input.to_path_buf());
    argv.extend([
        "-protocol_whitelist".to_string(),
        source.protocol_whitelist().to_string(),
        "-i".to_string(),
        source.ffmpeg_arg(),
    ]);

    // Shares the live path's video arguments, with a ladder of just this rate.
    let mut request = TranscodeRequest::new(
        source,
        output,
        TranscodePlan {
            segment_container: HlsSegmentContainer::Fmp4,
            video: target.video,
            audio: AudioPlan::None,
        },
        encoder,
        LadderEntry {
            // Unscaled: `bitrate_kbps` already suits the output size.
            max_pixels: 0,
            video_codec: VideoCodec::H264,
            video_profile: noflex_core::VideoProfile::H264High,
            audio_codec: AudioCodec::Aac,
            container: "mp4".to_string(),
            bitrate_kbps: target.bitrate_kbps,
        },
    );
    request.vaapi_device = vaapi_device.map(Path::to_path_buf);
    push_video(&mut argv, &request);

    for (output_index, track) in target.audio.iter().enumerate() {
        argv.extend(["-map".to_string(), format!("0:a:{}", track.stream)]);
        match track.plan {
            AudioPlan::None => {}
            AudioPlan::Copy { .. } => {
                argv.extend([format!("-c:a:{output_index}"), "copy".to_string()]);
            }
            AudioPlan::Encode { codec, channels } => {
                argv.extend([
                    format!("-c:a:{output_index}"),
                    audio_encoder_flag(codec).to_string(),
                    format!("-ac:a:{output_index}"),
                    channels.to_string(),
                    format!("-b:a:{output_index}"),
                    audio_bitrate_bps(codec, channels).to_string(),
                ]);
            }
        }
    }
    argv.extend([
        "-sn".to_string(),
        "-dn".to_string(),
        "-movflags".to_string(),
        PACKAGE_MOVFLAGS.to_string(),
        "-f".to_string(),
        "mp4".to_string(),
        output.display().to_string(),
    ]);
    argv
}

#[cfg(test)]
mod tests {
    use noflex_core::{ChannelLayout, HdrTransfer, PixelFormat, VideoProfile};

    use super::*;
    use crate::{
        decision::{AudioCodecSupport, HlsProfile, VideoCodecSupport},
        package::{TrackKind, finalize},
        probe::VideoStreamProbe,
    };

    fn audio(index: u32, codec: &str, channels: u8) -> AudioStreamProbe {
        AudioStreamProbe {
            index,
            codec: codec.into(),
            profile: None,
            bit_depth: None,
            sample_rate: Some(48_000),
            channels,
            channel_layout: Some(ChannelLayout::Stereo),
            language: None,
            default: index == 1,
            forced: false,
            title: None,
            audio_object_type: None,
        }
    }

    fn source(
        container: &str,
        codec: VideoCodec,
        profile: VideoProfile,
        height: u32,
        hdr: bool,
        audio: Vec<AudioStreamProbe>,
    ) -> ProbeReport {
        ProbeReport {
            container: Some(container.into()),
            video: Some(VideoStreamProbe {
                codec,
                profile: Some(profile),
                level: Some(120.0),
                width: height * 16 / 9,
                height,
                bit_depth: Some(if hdr { 10 } else { 8 }),
                pixel_format: Some(if hdr {
                    PixelFormat::Yuv420p10le
                } else {
                    PixelFormat::Yuv420p
                }),
                frame_rate: Some(24.0),
                color_space: None,
                transfer: hdr.then_some(HdrTransfer::Pq),
                matrix: None,
                dovi_profile: None,
                side_data: Default::default(),
            }),
            audio,
            subtitle: vec![],
            format_bit_rate: Some(12_000_000),
            duration_ms: Some(60_000),
        }
    }

    /// A device decoding `codecs` in fragmented MP4 up to `max_height`.
    fn device(codecs: &[VideoCodec], max_height: u32, hdr: bool) -> DeviceTarget {
        let profiles = |codec: &VideoCodec| match codec {
            VideoCodec::H265 => vec![VideoProfile::HevcMain10, VideoProfile::HevcMain],
            _ => vec![VideoProfile::H264High, VideoProfile::H264Main],
        };
        DeviceTarget {
            id: DeviceId::new(),
            capabilities: ClientCapabilities {
                direct_play: vec![],
                hls: vec![HlsProfile {
                    segment_container: HlsSegmentContainer::Fmp4,
                    video_codecs: codecs.to_vec(),
                    audio_codecs: vec![AudioCodec::Aac],
                }],
                video_codecs: codecs
                    .iter()
                    .map(|codec| VideoCodecSupport {
                        codec: *codec,
                        profiles: profiles(codec),
                        max_level: None,
                        max_bit_depth: Some(if hdr { 10 } else { 8 }),
                        max_width: None,
                        max_height: Some(max_height),
                        max_pixels: None,
                        video_ranges: if hdr { vec![VideoRange::Hdr10] } else { vec![] },
                    })
                    .collect(),
                audio_codecs: vec![AudioCodecSupport {
                    codec: AudioCodec::Aac,
                    max_channels: Some(6),
                }],
                subtitle_codecs: vec![],
                max_height: Some(max_height),
                max_bitrate_bps: None,
            },
        }
    }

    fn options() -> DecideOptions {
        DecideOptions {
            burn_subtitle: false,
            encode_video: vec![VideoCodec::H265, VideoCodec::H264],
            encode_audio: vec![AudioCodec::Aac],
            audio_stream: None,
        }
    }

    fn ladder() -> Vec<LadderEntry> {
        [(2_073_600, 8_000), (8_294_400, 20_000)]
            .map(|(max_pixels, bitrate_kbps)| LadderEntry {
                max_pixels,
                video_codec: VideoCodec::H264,
                video_profile: VideoProfile::H264High,
                audio_codec: AudioCodec::Aac,
                container: "mp4".to_string(),
                bitrate_kbps,
            })
            .to_vec()
    }

    fn codec_of(target: &OptimizeTarget) -> VideoCodec {
        match target.video {
            VideoPlan::Encode { codec, .. } | VideoPlan::Copy { codec, .. } => codec,
            VideoPlan::None => panic!("no video"),
        }
    }

    #[test]
    fn a_device_that_direct_plays_the_source_needs_nothing() {
        let probe = source(
            "mp4",
            VideoCodec::H264,
            VideoProfile::H264High,
            1080,
            false,
            vec![audio(1, "aac", 2)],
        );
        let mut web = device(&[VideoCodec::H264], 1080, false);
        web.capabilities.direct_play = vec![crate::decision::DirectPlayProfile {
            container: "mp4".to_string(),
            video_codecs: vec![VideoCodec::H264],
            audio_codecs: vec![AudioCodec::Aac],
        }];
        assert!(plan_versions(&probe, &[web], &options(), &ladder()).is_empty());
    }

    #[test]
    fn devices_at_one_height_unify_on_the_codec_they_all_decode() {
        let probe = source(
            "matroska",
            VideoCodec::Vp9,
            VideoProfile::Vp9Profile0,
            1080,
            false,
            vec![audio(1, "aac", 2)],
        );
        let h264_only = device(&[VideoCodec::H264], 1080, false);
        let both = device(&[VideoCodec::H265, VideoCodec::H264], 1080, false);
        let ids = [h264_only.id, both.id];

        let targets = plan_versions(&probe, &[h264_only, both], &options(), &ladder());

        assert_eq!(targets.len(), 1, "{targets:?}");
        assert_eq!(codec_of(&targets[0]), VideoCodec::H264);
        assert_eq!(targets[0].devices, ids);
    }

    #[test]
    fn devices_needing_different_quality_stay_separate() {
        let probe = source(
            "matroska",
            VideoCodec::H265,
            VideoProfile::HevcMain10,
            2160,
            true,
            vec![audio(1, "aac", 2)],
        );
        let tv = device(&[VideoCodec::H265, VideoCodec::H264], 2160, true);
        let phone = device(&[VideoCodec::H264], 1080, false);
        let (tv_id, phone_id) = (tv.id, phone.id);

        let targets = plan_versions(&probe, &[tv, phone], &options(), &ladder());

        assert_eq!(targets.len(), 2, "{targets:?}");
        assert!(
            matches!(targets[0].video, VideoPlan::Copy { .. }),
            "the TV keeps the HDR source"
        );
        assert_eq!(targets[0].devices, [tv_id]);
        assert!(matches!(
            targets[1].video,
            VideoPlan::Encode {
                codec: VideoCodec::H264,
                height: 1080,
                tonemap: true,
                ..
            }
        ));
        assert_eq!(targets[1].devices, [phone_id]);
    }

    #[test]
    fn every_audio_track_is_kept_and_copied_only_where_decodable() {
        let probe = source(
            "matroska",
            VideoCodec::Vp9,
            VideoProfile::Vp9Profile0,
            1080,
            false,
            vec![audio(1, "aac", 2), audio(2, "dts", 6), audio(3, "aac", 2)],
        );
        let targets = plan_versions(
            &probe,
            &[device(&[VideoCodec::H264], 1080, false)],
            &options(),
            &ladder(),
        );
        let tracks = &targets[0].audio;
        assert_eq!(tracks.len(), 3);
        assert_eq!(
            tracks[0].plan,
            AudioPlan::Copy {
                codec: AudioCodec::Aac
            }
        );
        assert_eq!(
            tracks[1].plan,
            AudioPlan::Encode {
                codec: AudioCodec::Aac,
                channels: 2
            }
        );
        assert_eq!(tracks[2].stream, 2);
    }

    #[test]
    fn a_device_limit_only_lowers_the_bitrate() {
        let probe = source(
            "matroska",
            VideoCodec::Vp9,
            VideoProfile::Vp9Profile0,
            1080,
            false,
            vec![audio(1, "aac", 2)],
        );
        let open = device(&[VideoCodec::H264], 1080, false);
        let mut limited = device(&[VideoCodec::H264], 1080, false);
        limited.capabilities.max_bitrate_bps = Some(3_000_000);

        let unlimited = plan_versions(&probe, &[open], &options(), &ladder());
        assert_eq!(unlimited[0].bitrate_kbps, 8_000);
        let capped = plan_versions(&probe, &[limited], &options(), &ladder());
        assert_eq!(capped[0].bitrate_kbps, 3_000);
        assert_ne!(unlimited[0].key(), capped[0].key());
    }

    #[test]
    fn built_arguments_produce_a_package_with_every_track() {
        let dir = std::env::temp_dir().join(format!("noflex-optimize-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "testsrc2=s=160x90:r=24:d=10"])
            .args(["-f", "lavfi", "-i", "sine=f=440:d=10"])
            .args(["-f", "lavfi", "-i", "sine=f=880:d=10"])
            .args(["-map", "0", "-map", "1", "-map", "2"])
            .args(["-c:v", "mpeg4", "-c:a", "mp3"])
            .arg(&input)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            eprintln!("ffmpeg unavailable; skipping");
            return;
        }
        let target = OptimizeTarget {
            video: VideoPlan::Encode {
                codec: VideoCodec::H264,
                width: 160,
                height: 90,
                tonemap: false,
                profile: VideoProfile::H264High,
            },
            bitrate_kbps: 300,
            audio: vec![
                AudioTarget {
                    stream: 0,
                    plan: AudioPlan::Encode {
                        codec: AudioCodec::Aac,
                        channels: 2,
                    },
                },
                AudioTarget {
                    stream: 1,
                    plan: AudioPlan::Copy {
                        codec: AudioCodec::Mp3,
                    },
                },
            ],
            devices: vec![],
        };
        let raw = dir.join("raw.mp4");
        let argv = build_optimize_argv(&input, &target, Encoder::Software264, None, &raw);
        let output = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let index = finalize(&raw, &dir.join("package.mp4")).unwrap();
        let kinds = index.tracks.iter().map(|t| t.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [TrackKind::Video, TrackKind::Audio, TrackKind::Audio]
        );
        let longest = index.tracks[0]
            .fragments
            .iter()
            .map(|fragment| fragment.duration)
            .max()
            .unwrap();
        let seconds = longest as f64 / f64::from(index.tracks[0].timescale);
        assert!((seconds - 4.0).abs() < 0.3, "keyframes every 4s: {seconds}");

        std::fs::remove_dir_all(dir).ok();
    }
}
