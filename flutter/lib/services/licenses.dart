// SPDX-License-Identifier: AGPL-3.0-only
//
// Flutter's license registry knows every Dart package, but not the native
// libraries media_kit downloads prebuilt for each platform, nor the bundled font.
// This adds them for the platform this build is for. The lists follow the builds
// media_kit's packages pin (see pubspec.yaml), so a change there comes here too:
// another library, a library dropped, or a new version.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

class _Library {
  const _Library(this.name, this.version, this.notice, [this.texts = const []]);

  final String name;
  final String? version;

  /// Its copyright and terms, in assets/licenses/.
  final String notice;

  /// The full license texts its notice names, also in assets/licenses/.
  final List<String> texts;
}

const _lgpl2 = ['LGPL-2.1'];
const _lgpl3 = ['LGPL-3.0', 'GPL-3.0'];
const _apache = ['Apache-2.0'];

const _everywhere = [_Library('Inter', null, 'inter')];

// libmpv-darwin-build v0.7.2, video-default.
const _apple = [
  _Library('mpv', '0.36.0', 'mpv', _lgpl2),
  _Library('FFmpeg', '6.0', 'ffmpeg', _lgpl3),
  _Library('libass', '0.17.1', 'libass'),
  _Library('FreeType', '2.13.2', 'freetype'),
  _Library('GNU FriBidi', '1.0.13', 'fribidi', _lgpl2),
  _Library('HarfBuzz', '8.1.1', 'harfbuzz'),
  _Library('Mbed TLS', '3.4.1', 'mbedtls', _apache),
  _Library('dav1d', '1.2.1', 'dav1d'),
  _Library('libpng', '1.6.40', 'libpng'),
  _Library('uchardet', '0.0.8', 'uchardet', _lgpl2),
  _Library('libxml2', '2.11.5', 'libxml2'),
];

// libmpv-android-video-build v1.1.7, default.
const _android = [
  _Library('mpv', '78d4374', 'mpv', _lgpl2),
  _Library('FFmpeg', '6.0', 'ffmpeg', _lgpl3),
  _Library('libass', '0.17.1', 'libass'),
  _Library('FreeType', '2.13.0', 'freetype'),
  _Library('GNU FriBidi', '1.0.12', 'fribidi', _lgpl2),
  _Library('HarfBuzz', '7.2.0', 'harfbuzz'),
  _Library('Mbed TLS', '3.4.0', 'mbedtls', _apache),
  _Library('dav1d', '1.2.0', 'dav1d'),
  _Library('libxml2', '2.10.3', 'libxml2'),
];

// libmpv-win32-video-cmake 20241021: what mpv's enabled features and FFmpeg's
// configuration link into the one DLL, read from the DLL itself. Most of it
// builds from a branch head, so there's no version to give. libarchive is
// configured with LZO, which is GPL, but none of it is in the DLL.
const _windows = [
  _Library('mpv', '0.39.0', 'mpv', _lgpl2),
  _Library('FFmpeg', null, 'ffmpeg', _lgpl3),
  _Library('libass', null, 'libass'),
  _Library('FreeType', null, 'freetype'),
  _Library('GNU FriBidi', null, 'fribidi', _lgpl2),
  _Library('HarfBuzz', null, 'harfbuzz'),
  _Library('Fontconfig', null, 'fontconfig'),
  _Library('Expat', null, 'expat'),
  _Library('libunibreak', null, 'libunibreak'),
  _Library('GNU libiconv', null, 'libiconv', _lgpl2),
  _Library('libplacebo', null, 'libplacebo', _lgpl2),
  _Library('Shaderc', null, 'shaderc', _apache),
  _Library('glslang', null, 'glslang'),
  _Library('SPIRV-Tools', null, 'spirv-tools', _apache),
  _Library('SPIRV-Cross', null, 'spirv-cross', _apache),
  _Library('Vulkan Loader', null, 'vulkan-loader'),
  _Library('Little CMS', null, 'lcms2'),
  _Library('libjpeg-turbo', null, 'libjpeg-turbo'),
  _Library('libpng', null, 'libpng'),
  _Library('zlib', null, 'zlib'),
  _Library('uchardet', null, 'uchardet', _lgpl2),
  _Library('libarchive', null, 'libarchive'),
  _Library('XZ Utils', null, 'xz'),
  _Library('Zstandard', null, 'zstd'),
  _Library('bzip2', null, 'bzip2'),
  _Library('libxml2', null, 'libxml2'),
  _Library('OpenSSL', null, 'openssl', _apache),
  _Library('libssh', null, 'libssh', _lgpl2),
  _Library('libjxl', null, 'libjxl'),
  _Library('Highway', null, 'highway'),
  _Library('Brotli', null, 'brotli'),
  _Library('libbluray', '1.3.4', 'libbluray', _lgpl2),
  _Library('libudfread', null, 'libudfread', _lgpl2),
  _Library('zimg', null, 'zimg'),
  _Library('libdovi', null, 'libdovi'),
  _Library('xxHash', null, 'xxhash'),
  _Library('fast_float', null, 'fast_float'),
  _Library('dav1d', null, 'dav1d'),
  _Library('ANGLE', null, 'angle'),
];

// Linux plays through the system's libmpv, which the app doesn't ship.
const _linux = [_Library('mimalloc', '2.1.2', 'mimalloc')];

List<_Library> get _libraries => [
  ..._everywhere,
  ...switch (defaultTargetPlatform) {
    TargetPlatform.iOS || TargetPlatform.macOS => _apple,
    TargetPlatform.android => _android,
    TargetPlatform.windows => _windows,
    TargetPlatform.linux => _linux,
    TargetPlatform.fuchsia => const <_Library>[],
  },
];

void registerNativeLicenses() => LicenseRegistry.addLicense(_entries);

Stream<LicenseEntry> _entries() async* {
  Future<String> load(String name) =>
      rootBundle.loadString('assets/licenses/$name.txt');
  for (final library in _libraries) {
    final parts = [
      if (library.version case final version?) 'Version $version.',
      await load(library.notice),
      for (final text in library.texts) await load(text),
    ];
    yield LicenseEntryWithLineBreaks([library.name], parts.join('\n\n'));
  }
}

/// Everything this build carries, by what it's called, in alphabetical order:
/// Dart packages, Flutter's engine, and the libraries above.
Future<List<(String, List<LicenseEntry>)>> licensesByPackage() async {
  final byPackage = <String, List<LicenseEntry>>{};
  await for (final entry in LicenseRegistry.licenses) {
    for (final package in entry.packages) {
      (byPackage[package] ??= []).add(entry);
    }
  }
  return [for (final MapEntry(:key, :value) in byPackage.entries) (key, value)]
    ..sort((a, b) => a.$1.toLowerCase().compareTo(b.$1.toLowerCase()));
}
