# SPDX-License-Identifier: AGPL-3.0-only

# List Flutter app recipes and their descriptions.
default:
    @just --list app

# Fetch dependencies and generate GraphQL types; a fresh checkout needs this first.
[working-directory('../flutter')]
setup:
    flutter pub get
    dart run build_runner build

# Regenerate GraphQL types, after any *.graphql or schema change.
[working-directory('../flutter')]
gen:
    dart run build_runner build

# Run the app on a device (default: the Linux desktop).
[working-directory('../flutter')]
run device='linux' *args:
    flutter run -d {{ device }} {{ args }}

# Run the app on a running emulator, with the video output emulators need.
[working-directory('../flutter')]
run-emulator device='emulator-5554' *args:
    flutter run -d {{ device }} --dart-define=NOFLEX_ANDROID_VO=mediacodec_embed {{ args }}

# Analyze the app; must be clean before committing, infos included.
[working-directory('../flutter')]
analyze:
    flutter analyze --no-pub

# Run the app's tests; extra arguments pick files or filter by name.
[working-directory('../flutter')]
test *args:
    flutter test --no-pub {{ args }}

# Format the app (2-space, unlike the rest of the repo).
[working-directory('../flutter')]
fmt:
    dart format .

# Regenerate assets/decoder-probes/, the clips that measure what a device decodes in hardware.
[working-directory('../flutter')]
probes:
    tool/make_decoder_probes.sh

# Create the Android foldable test device, in this checkout's .cache/android/avd.
avd-create name='noflex_fold' device='pixel_fold':
    avdmanager create avd -n {{ name }} -k "system-images;android-36;google_apis_playstore;x86_64" --device {{ device }}

# Boot an Android test device; pass -no-window to run it headless. The host is 10.0.2.2 from inside it.
emulator name='noflex_fold' *args:
    emulator -avd {{ name }} -gpu host -no-boot-anim {{ args }}
