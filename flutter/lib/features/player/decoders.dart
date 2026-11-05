// SPDX-License-Identifier: AGPL-3.0-only
//
// What this device decodes, as the player measured it, and the two decisions
// that follow: which version of a title to prefer, and what to tell the server.
// Pure, so the policy is testable without a device.

import '../../models/media_probe.dart';

/// How a device would decode a picture, best first.
enum DecodeTier {
  /// Needs the server to transcode, or can't keep up.
  server,
  software,
  hardware,
}

/// A picture size and depth that decoded.
class DecodeCeiling {
  const DecodeCeiling(this.pixels, this.bitDepth);

  factory DecodeCeiling.fromJson(List<dynamic> json) =>
      DecodeCeiling(json[0] as int, json[1] as int);

  final int pixels;
  final int bitDepth;

  bool covers(int pixels, int bitDepth) =>
      pixels <= this.pixels && bitDepth <= this.bitDepth;

  List<int> toJson() => [pixels, bitDepth];
}

const pixels1080p = 1920 * 1080;
const pixels2160p = 3840 * 2160;

/// The server's video codec names, in the spelling the probe report uses.
const measuredCodecs = ['h264', 'h265', 'av1', 'vp9'];

class DecoderProfile {
  const DecoderProfile({
    this.hardware = const {},
    required this.softwareCeiling,
    this.audioDecoders = const {},
    this.hwdecApi,
  });

  factory DecoderProfile.fromJson(Map<String, dynamic> json) => DecoderProfile(
    hardware: (json['hardware'] as Map).cast<String, dynamic>().map(
      (codec, list) => MapEntry(codec, [
        for (final entry in list as List) DecodeCeiling.fromJson(entry as List),
      ]),
    ),
    softwareCeiling: DecodeCeiling.fromJson(json['software'] as List),
    audioDecoders: Set<String>.from(json['audio'] as List),
    hwdecApi: json['api'] as String?,
  );

  /// Per codec, every size and depth that decoded in hardware.
  final Map<String, List<DecodeCeiling>> hardware;

  /// The most software can do in real time here. Unlike hardware this isn't
  /// measured per codec: dav1d and friends scale with the CPU, and a clip of
  /// black frames says nothing about a film.
  final DecodeCeiling softwareCeiling;

  /// libmpv's audio decoders by codec name. The bundles differ: Windows has no
  /// TrueHD, for one.
  final Set<String> audioDecoders;

  /// libmpv's name for what it used, such as `mediacodec-copy`.
  final String? hwdecApi;

  Map<String, dynamic> toJson() => {
    'hardware': hardware.map(
      (codec, list) => MapEntry(codec, [for (final c in list) c.toJson()]),
    ),
    'software': softwareCeiling.toJson(),
    'audio': audioDecoders.toList(),
    'api': hwdecApi,
  };

  DecodeTier tierOf(VideoProbe video) {
    final depth = video.bitDepth ?? 8;
    final ceilings = hardware[video.codec] ?? const [];
    if (ceilings.any((c) => c.covers(video.pixels, depth))) {
      return DecodeTier.hardware;
    }
    return softwareCeiling.covers(video.pixels, depth)
        ? DecodeTier.software
        : DecodeTier.server;
  }

  /// A file played in software despite the measurement: trust what happened.
  /// A real film at a higher level or profile can miss where a probe clip hit.
  DecoderProfile withoutHardware(VideoProbe video) {
    final depth = video.bitDepth ?? 8;
    final remaining = [
      for (final ceiling in hardware[video.codec] ?? const <DecodeCeiling>[])
        if (ceiling.pixels < video.pixels || ceiling.bitDepth < depth) ceiling,
    ];
    return DecoderProfile(
      hardware: {...hardware, video.codec: remaining},
      softwareCeiling: softwareCeiling,
      audioDecoders: audioDecoders,
      hwdecApi: hwdecApi,
    );
  }
}

/// One file of an edition, as version selection sees it.
class MediaVersion {
  const MediaVersion({
    required this.id,
    required this.editionId,
    required this.partIndex,
    required this.sizeBytes,
    required this.probe,
    this.path = '',
    this.container,
  });

  final String id;
  final String editionId;

  /// Files sharing a part index are alternate versions; distinct indexes are
  /// parts of one long cut.
  final int partIndex;
  final int sizeBytes;
  final MediaProbe probe;
  final String path;
  final String? container;

  String? get label => probe.versionLabel;
  String get fileName => path.split(RegExp(r'[/\\]')).last;
}

/// The versions of one part, best for this device first: what decodes in
/// hardware, then the most picture the display can show, then SDR over an HDR
/// master (the player has to tone-map that), then whatever costs less.
List<MediaVersion> rankVersions(
  List<MediaVersion> versions,
  DecoderProfile profile, {
  required int displayPixels,
}) {
  int tier(MediaVersion v) => v.probe.video == null
      ? DecodeTier.software.index
      : profile.tierOf(v.probe.video!).index;
  int shown(MediaVersion v) {
    final pixels = v.probe.video?.pixels ?? 0;
    return pixels < displayPixels ? pixels : displayPixels;
  }

  final ranked = [...versions];
  ranked.sort((a, b) {
    final byTier = tier(b).compareTo(tier(a));
    if (byTier != 0) return byTier;
    final byPicture = shown(b).compareTo(shown(a));
    if (byPicture != 0) return byPicture;
    final byHdr = (a.probe.video?.isHdr == true ? 1 : 0).compareTo(
      b.probe.video?.isHdr == true ? 1 : 0,
    );
    if (byHdr != 0) return byHdr;
    final byPixels = (a.probe.video?.pixels ?? 0).compareTo(
      b.probe.video?.pixels ?? 0,
    );
    if (byPixels != 0) return byPixels;
    return a.sizeBytes.compareTo(b.sizeBytes);
  });
  return ranked;
}

/// The first part's best version for each edition.
class EditionChoice {
  const EditionChoice(this.editionId, this.best, this.tier);
  final String editionId;
  final MediaVersion? best;
  final DecodeTier tier;
}

/// Which edition to open when the viewer hasn't said: the first in the server's
/// order that can play in hardware, else the first that can play at all. An
/// edition is a different cut, so order breaks ties rather than picture size.
EditionChoice? pickEdition(
  List<(String editionId, List<MediaVersion> versions)> editions,
  DecoderProfile profile, {
  required int displayPixels,
}) {
  EditionChoice? best;
  for (final (editionId, versions) in editions) {
    final firstPart = firstPartOf(versions);
    if (firstPart.isEmpty) continue;
    final top = rankVersions(
      firstPart,
      profile,
      displayPixels: displayPixels,
    ).first;
    final tier = top.probe.video == null
        ? DecodeTier.hardware
        : profile.tierOf(top.probe.video!);
    if (best == null || tier.index > best.tier.index) {
      best = EditionChoice(editionId, top, tier);
    }
  }
  return best;
}

List<MediaVersion> firstPartOf(List<MediaVersion> versions) {
  if (versions.isEmpty) return const [];
  final first = versions
      .map((v) => v.partIndex)
      .reduce((a, b) => a < b ? a : b);
  return [
    for (final version in versions)
      if (version.partIndex == first) version,
  ];
}
