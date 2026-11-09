// SPDX-License-Identifier: AGPL-3.0-only
//
// Settings: a list of sections, the chosen one beside it. What follows the viewer is
// the `playback` preference every client shares; what belongs to this TV stays on it.
// Profile, sessions and server administration are easier with a real keyboard, and stay
// in the web client and the apps.

import { useNavigate } from "@solidjs/router";
import { deviceBrand, type DeviceBrand } from "$devices";
import { Column, Row } from "@solidtv/solid/primitives";
import { createEffect, createMemo, createSignal, For, Index, onCleanup, Show, untrack } from "solid-js";
import type { ElementNode, NodeProps } from "@solidtv/solid";
import { bytes, languageName } from "$lib/format.js";
import { MY_DEVICES, MY_SMART_MARKS, PREPARED_VERSIONS, UNMARK_FOR_SMART_TRANSCODING, UPDATE_DEVICE } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import { host } from "$lib/host.js";
import type { IconName } from "$lib/icons.generated.js";
import { app, changeServer, logout, savePlaybackPrefs, setShowLibraries, switchProfile, thisDeviceId } from "$lib/session.js";
import { changeSetting, settings, type DeviceSettings } from "$lib/settings.js";
import { serverOrigin } from "$lib/tokens.js";
import type { Choice } from "$lib/tracks.js";
import type { Device, PreparedVersion, SmartMark, SmartMarkState } from "$lib/types.js";
import { attempt } from "$lib/ui.js";
import { color, motion, PAGE_WIDTH, radius, SCREEN, type, weight } from "../theme.js";
import Avatar from "$components/Avatar.js";
import Button from "$components/Button.js";
import Icon from "$components/Icon.js";
import Page, { BODY_HEIGHT } from "$components/Page.js";
import QrCode from "$components/QrCode.js";
import SettingRow from "$components/SettingRow.js";
import Sheet from "$components/Sheet.js";

const SECTIONS_WIDTH = 340;
const GAP = 48;
const PANEL_WIDTH = PAGE_WIDTH - SECTIONS_WIDTH - GAP - 64;
const ROW_GAP = 10;
const MARK_POLL_MS = 5000;

type Section = "playback" | "player" | "tv" | "smart" | "account";
const SECTIONS: { id: Section; label: string; icon: IconName }[] = [
  { id: "playback", label: "Playback", icon: "play" },
  { id: "player", label: "Player", icon: "sliders-horizontal" },
  { id: "tv", label: "This TV", icon: "television" },
  { id: "smart", label: "Smart transcoding", icon: "sparkle" },
  { id: "account", label: "Account", icon: "user" },
];

const QUALITIES: Choice[] = [
  { value: "auto", label: "Best this TV can play" },
  { value: "2160", label: "Up to 4K" },
  { value: "1080", label: "Up to 1080p" },
  { value: "720", label: "Up to 720p" },
  { value: "480", label: "Up to 480p", hint: "Data saver" },
];
const LANGUAGES = ["en", "es", "fr", "de", "it", "pt", "nl", "sv", "pl", "ru", "ja", "ko", "zh", "hi", "ar", "tr"];
const languageChoices = (none: string): Choice[] => [{ value: "", label: none }, ...LANGUAGES.map((code) => ({ value: code, label: languageName(code) }))];
const PREPARE: Choice[] = [
  { value: "", label: "Best it can play" },
  { value: "20000000", label: "Up to 20 Mbps" },
  { value: "8000000", label: "Up to 8 Mbps" },
  { value: "4000000", label: "Up to 4 Mbps" },
];
const MARK_STATES: Record<SmartMarkState, string> = {
  WAITING: "In the queue",
  PREPARING: "Preparing",
  READY: "Ready",
  NOT_NEEDED: "Plays as it is",
  TOO_LARGE: "Too large for this server",
  NO_DEVICES: "No device has it on",
  NO_SPACE: "Waiting for space",
  FAILED: "Failed. Unmark to retry",
  DISABLED: "Off on this server",
};
const SKIPS: Choice[] = [5, 10, 15, 30].map((seconds) => ({ value: String(seconds), label: `${seconds} seconds` }));
const COUNTDOWNS: Choice[] = [5, 10, 20, 30].map((seconds) => ({ value: String(seconds), label: `${seconds} seconds` }));
const CAPTION_SIZES: Choice[] = [{ value: "small", label: "Small" }, { value: "medium", label: "Medium" }, { value: "large", label: "Large" }];
const CAPTION_BACKS: Choice[] = [{ value: "none", label: "None" }, { value: "dim", label: "Dimmed" }, { value: "solid", label: "Solid" }];

const labelOf = (choices: Choice[], value: string) => choices.find((choice) => choice.value === value)?.label ?? value;
// A language picked here goes first; any others set from a browser stay behind it.
const putFirst = (list: string[], first: string) => (first ? [first, ...list.filter((item) => item !== first)] : []);

interface Picker { title: string; choices: Choice[]; value: string; pick: (value: string) => void }

function SectionItem(props: NodeProps & { section: (typeof SECTIONS)[number]; chosen: boolean; onChosen: () => void }) {
  const [focused, setFocused] = createSignal(false);
  const ink = () => (focused() ? color.lightInk : props.chosen ? color.accent : color.text2);
  return (
    <view
      {...props}
      width={SECTIONS_WIDTH}
      height={72}
      borderRadius={radius.m}
      color={focused() ? color.light : props.chosen ? color.surface2 : color.clear}
      transition={{ color: motion.fast }}
      onFocusChanged={(has: boolean) => { setFocused(has); if (has) props.onChosen(); }}
    >
      <Icon name={props.section.icon} size={32} x={20} y={20} color={ink()} />
      <text x={68} height={72} lineHeight={72} fontSize={type.caption} fontWeight={weight.semibold} color={ink()}>{props.section.label}</text>
    </view>
  );
}

const PROFILE_QR = 170;

const BRAND_ICONS: Record<DeviceBrand, IconName> = {
  android: "android-logo-fill", apple: "apple-logo-fill", archlinux: "mark-archlinux", brave: "mark-brave", browser: "browser", chrome: "mark-chrome", debian: "mark-debian", desktop: "desktop",
  duckduckgo: "mark-duckduckgo", fedora: "mark-fedora", firefox: "mark-firefox", gamepad: "game-controller", googletv: "mark-googletv", laptop: "laptop", linux: "linux-logo-fill", linuxmint: "mark-linuxmint",
  manjaro: "mark-manjaro", nixos: "mark-nixos", nvidia: "mark-nvidia", opensuse: "mark-opensuse", opera: "mark-opera", phone: "device-mobile", playstation: "mark-playstation", popos: "mark-popos",
  safari: "mark-safari", steamdeck: "mark-steamdeck", tablet: "device-tablet", television: "television", ubuntu: "mark-ubuntu", vivaldi: "mark-vivaldi", windows: "windows-logo-fill",
};

const Note = (props: { text: string }) => (
  <text skipFocus width={PANEL_WIDTH} contain="width" lineHeight={32} fontSize={22} color={color.text3}>{props.text}</text>
);

const Heading = (props: { text: string }) => (
  <text skipFocus height={44} lineHeight={44} fontSize={type.caption} fontWeight={weight.semibold} color={color.text2}>{props.text}</text>
);

export default function Account() {
  const [section, setSection] = createSignal<Section>("playback");
  const [picker, setPicker] = createSignal<Picker | null>(null);
  const [devices, setDevices] = createSignal<Device[] | null>(null);
  const [marks, setMarks] = createSignal<SmartMark[]>([]);
  const [prepared, setPrepared] = createSignal<PreparedVersion[]>([]);
  const navigate = useNavigate();

  createEffect(() => {
    void request<{ myDevices: Device[] }>(MY_DEVICES).then((data) => setDevices(data.myDevices), () => setDevices([]));
  });

  const loadMarks = () => Promise.all([
    request<{ mySmartTranscodeMarks: SmartMark[] }>(MY_SMART_MARKS).then((data) => setMarks(data.mySmartTranscodeMarks), () => {}),
    request<{ preparedVersions: PreparedVersion[] }>(PREPARED_VERSIONS).then((data) => setPrepared(data.preparedVersions), () => {}),
  ]);
  createEffect(() => { if (section() === "smart") void loadMarks(); });
  // Only while it's on screen and something is moving: a TV left here should idle.
  const marksMoving = createMemo(() => section() === "smart" && marks().some((mark) => mark.state === "WAITING" || mark.state === "PREPARING"));
  createEffect(() => {
    if (!marksMoving()) return;
    const timer = setInterval(() => void loadMarks(), MARK_POLL_MS);
    onCleanup(() => clearInterval(timer));
  });
  let panel: ElementNode | undefined;
  const unmark = (mark: SmartMark) => attempt(async () => {
    await request(UNMARK_FOR_SMART_TRANSCODING, { work: mark.work.id, device: mark.device?.id ?? null });
    setMarks((list) => list.filter((item) => item.work.id !== mark.work.id || item.device?.id !== mark.device?.id));
    // The last row may have gone from under focus; settle on whatever now stands there.
    if (!panel) return;
    let index = Math.min(panel.selected ?? 0, panel.children.length - 1);
    while (index > 0 && panel.children[index]?.skipFocus) index--;
    panel.selected = index;
    panel.setFocus();
  });

  const prefs = () => app.playbackPrefs;
  const here = () => devices()?.find((device) => device.id === thisDeviceId());

  const savePlayback = (change: (stored: any) => any) => attempt(() => savePlaybackPrefs(change));
  const saveDevice = (device: Device, change: Record<string, unknown>) => attempt(async () => {
    const data = await request<{ updateDevice: Device }>(UPDATE_DEVICE, { id: device.id, ...change });
    setDevices((list) => (list ?? []).map((item) => (item.id === device.id ? { ...item, ...data.updateDevice } : item)));
  });
  // OK on a row opens it, as on every other row with a chevron: leaving the list is a choice.
  const markOptions = (mark: SmartMark) => choose(mark.work.title, [
    { value: "open", label: "Go to this title" },
    { value: "unmark", label: "Unmark", hint: "Keeps what’s already prepared" },
  ], "", (value) => {
    if (value === "open") navigate(`/works/${mark.work.id}`);
    else void unmark(mark);
  });
  const choose = (title: string, choices: Choice[], value: string, pick: (value: string) => void) => {
    setPicker({ title, choices, value, pick });
    return true;
  };
  const device = <K extends keyof DeviceSettings>(key: K, value: DeviceSettings[K]) => changeSetting(key, value);

  const panels: Record<Section, () => any> = {
    playback: () => (
      <>
        <Note text="These follow you to every device you sign in on." />
        <SettingRow width={PANEL_WIDTH} label="Streaming quality" detail="Lower this if playback stalls" value={labelOf(QUALITIES, QUALITIES.some((choice) => choice.value === prefs().quality) ? prefs().quality : "auto")} onEnter={() => choose("Streaming quality", QUALITIES, prefs().quality, (value) => void savePlayback((stored) => ({ ...stored, quality: value })))} />
        <SettingRow width={PANEL_WIDTH} label="Play the next episode automatically" detail="When the credits start" checked={prefs().autoplayNext} onEnter={() => { void savePlayback((stored) => ({ ...stored, autoplayNext: !prefs().autoplayNext })); return true; }} />
        <SettingRow width={PANEL_WIDTH} label="Audio language" detail="When a title has a choice" value={prefs().audioLanguages[0] ? languageName(prefs().audioLanguages[0]) : "The title’s default"} onEnter={() => choose("Audio language", languageChoices("The title’s default"), prefs().audioLanguages[0] ?? "", (value) => void savePlayback((stored) => ({ ...stored, audio: { ...stored.audio, languages: putFirst(prefs().audioLanguages, value) } })))} />
        <SettingRow width={PANEL_WIDTH} label="Start with subtitles on" checked={prefs().subtitlesOn} onEnter={() => { void savePlayback((stored) => ({ ...stored, subtitles: { ...stored.subtitles, enabled: !prefs().subtitlesOn } })); return true; }} />
        <SettingRow width={PANEL_WIDTH} label="Subtitle language" value={prefs().subtitleLanguages[0] ? languageName(prefs().subtitleLanguages[0]) : "The title’s default"} onEnter={() => choose("Subtitle language", languageChoices("The title’s default"), prefs().subtitleLanguages[0] ?? "", (value) => void savePlayback((stored) => ({ ...stored, subtitles: { ...stored.subtitles, languages: putFirst(prefs().subtitleLanguages, value) } })))} />
      </>
    ),
    player: () => (
      <>
        <Note text="These stay on this TV." />
        <SettingRow width={PANEL_WIDTH} label="Skip intros automatically" detail="Where the server has found one" checked={settings.autoSkipIntro} onEnter={() => { device("autoSkipIntro", !settings.autoSkipIntro); return true; }} />
        <SettingRow width={PANEL_WIDTH} label="Skip by" detail="Each press of Left or Right" value={labelOf(SKIPS, String(settings.skipStep))} onEnter={() => choose("Skip by", SKIPS, String(settings.skipStep), (value) => device("skipStep", Number(value)))} />
        <SettingRow width={PANEL_WIDTH} label="Wait before the next episode" value={labelOf(COUNTDOWNS, String(settings.upNextSeconds))} onEnter={() => choose("Wait before the next episode", COUNTDOWNS, String(settings.upNextSeconds), (value) => device("upNextSeconds", Number(value)))} />
        <SettingRow width={PANEL_WIDTH} label="Subtitle size" value={labelOf(CAPTION_SIZES, settings.captionSize)} onEnter={() => choose("Subtitle size", CAPTION_SIZES, settings.captionSize, (value) => device("captionSize", value as DeviceSettings["captionSize"]))} />
        <SettingRow width={PANEL_WIDTH} label="Subtitle background" value={labelOf(CAPTION_BACKS, settings.captionBackground)} onEnter={() => choose("Subtitle background", CAPTION_BACKS, settings.captionBackground, (value) => device("captionBackground", value as DeviceSettings["captionBackground"]))} />
      </>
    ),
    tv: () => (
      <>
        <view skipFocus width={PANEL_WIDTH} height={92}>
          <Icon name={BRAND_ICONS[deviceBrand(host.deviceName(), host.clientName)]} size={56} y={10} />
          <text x={80} y={6} width={PANEL_WIDTH - 80} contain="width" maxLines={1} fontSize={type.lead} fontWeight={weight.semibold}>{host.deviceName()}</text>
          <text x={80} y={50} fontSize={type.caption} color={color.text2}>How this TV shows up in your device list</text>
        </view>
        <SettingRow width={PANEL_WIDTH} label="HDR" detail="Turn off if HDR looks wrong here; the server converts it to SDR" checked={settings.allowHdr} onEnter={() => { device("allowHdr", !settings.allowHdr); return true; }} />
      </>
    ),
    smart: () => (
      <>
        <Show when={here()} fallback={<Note text="This TV hasn’t registered with the server yet. It does when it starts." />}>
          {(me) => (
            <>
              <SettingRow width={PANEL_WIDTH} label="Smart transcoding on this TV" detail="While it’s idle, the server prepares what you’ll likely watch here" checked={me().smartTranscode} onEnter={() => { void saveDevice(me(), { smartTranscode: !me().smartTranscode }); return true; }} />
              <SettingRow width={PANEL_WIDTH} label="Quality of prepared versions" value={labelOf(PREPARE, String(me().maxBitrateBps ?? ""))} onEnter={() => choose("Quality of prepared versions", PREPARE, String(me().maxBitrateBps ?? ""), (value) => void saveDevice(me(), value ? { maxBitrateBps: Number(value) } : { clearMaxBitrate: true }))} />
            </>
          )}
        </Show>
        <Heading text="Marked to watch next" />
        <Show when={marks().length} fallback={<Note text="Nothing yet. Mark a title from its page, and it’s prepared ahead of anything the server guesses at." />}>
          <Note text="These are prepared first, in this order, until you watch them." />
          {/* By position: each load brings new objects, and a row holding focus has to survive them. */}
          <Index each={marks()}>
            {(mark) => (
              <SettingRow
                width={PANEL_WIDTH}
                label={mark().work.title}
                detail={mark().device ? `For ${mark().device!.name} only` : "For your devices with smart transcoding on"}
                value={mark().state === "PREPARING" ? `Preparing ${Math.round(mark().progress * 100)}%` : MARK_STATES[mark().state]}
                onEnter={() => markOptions(mark())}
              />
            )}
          </Index>
        </Show>
        <Show when={prepared().length}>
          <Heading text="Ready to play" />
          <Note text="Already prepared on this server, and used by any device it suits. Unmarking a title doesn’t remove it." />
          <Index each={prepared()}>
            {(version) => (
              <SettingRow
                width={PANEL_WIDTH}
                label={version().work?.title ?? "A removed title"}
                detail={version().description}
                value={bytes(version().sizeBytes)}
                onEnter={() => { const id = version().work?.id; if (id) navigate(`/works/${id}`); return true; }}
              />
            )}
          </Index>
        </Show>
      </>
    ),
    account: () => (
      <>
        <view skipFocus width={PANEL_WIDTH} height={PROFILE_QR + 40}>
          <Avatar y={8} size={120} name={app.viewer?.name ?? ""} url={app.viewer?.avatarUrl} />
          <text x={150} y={18} width={PANEL_WIDTH - PROFILE_QR - 190} contain="width" maxLines={1} fontSize={type.title} fontWeight={weight.semibold}>{app.viewer?.name ?? ""}</text>
          <text x={150} y={76} width={PANEL_WIDTH - PROFILE_QR - 190} contain="width" maxLines={1} fontSize={type.caption} color={color.text2}>{`${app.viewer?.isAdmin ? "Administrator" : "Member"} · ${app.server?.name ?? "NoFlex"} v${app.server?.version ?? ""}`}</text>
          <text x={150} y={110} width={PANEL_WIDTH - PROFILE_QR - 190} contain="width" maxLines={1} fontSize={22} color={color.text3}>{`Signs in as ${app.viewer?.username ?? ""} · ${serverOrigin() ?? ""}`}</text>
          <QrCode x={PANEL_WIDTH - PROFILE_QR} text={`${serverOrigin() ?? ""}/account?tab=profile`} size={PROFILE_QR} />
          <text x={PANEL_WIDTH - PROFILE_QR} y={PROFILE_QR + 6} width={PROFILE_QR} contain="width" textAlign="center" fontSize={20} color={color.text3}>Scan to edit</text>
        </view>
        <SettingRow width={PANEL_WIDTH} label="Show libraries in the navigation" detail="Alongside Movies, TV Shows and the rest" checked={app.showLibraries} onEnter={() => { void attempt(() => setShowLibraries(!app.showLibraries)); return true; }} />
        <Row width={PANEL_WIDTH} height={84} gap={20} scroll="none">
          <Button size="lg" icon="user-switch" label="Switch profile" onEnter={() => { void switchProfile(); return true; }} />
          <Button size="lg" icon="sign-out" label="Sign out" onEnter={() => { void attempt(logout); return true; }} />
          <Button size="lg" icon="globe" label="Change server" onEnter={() => { changeServer(); return true; }} />
        </Row>
        <Note text={app.viewer?.hasPin
          ? "Switching profile locks yours behind your PIN. Signing out takes only you off this TV; changing server signs everyone out."
          : "Signing out takes only you off this TV; changing server signs everyone out. A PIN keeps your profile yours here: set one under Profile, in the web client or the apps."} />
        <Note text="Your name, picture, PIN, devices and server administration are in the web client and the apps. The code opens your profile on a phone." />
      </>
    ),
  };

  return (
    <>
      <Page title="Settings">
        <Row autofocus width={PAGE_WIDTH} height={BODY_HEIGHT} gap={GAP} scroll="none">
          <Column width={SECTIONS_WIDTH} height={SECTIONS.length * 80} gap={8} scroll="none">
            <For each={SECTIONS}>{(item) => <SectionItem section={item} chosen={section() === item.id} onChosen={() => setSection(item.id)} />}</For>
          </Column>
          <view width={PANEL_WIDTH} height={BODY_HEIGHT} forwardFocus={0}>
            <Show when={section()} keyed>
              {(chosen) => (
                <Column ref={(node: ElementNode) => { panel = node; }} width={PANEL_WIDTH} height={SCREEN.height * 2} gap={ROW_GAP} scroll="auto">
                  {untrack(panels[chosen])}
                </Column>
              )}
            </Show>
          </view>
        </Row>
      </Page>
      <Show when={picker()} keyed>
        {(open) => (
          <Sheet title={open.title} choices={open.choices} value={open.value} onPick={(value) => { open.pick(value); setPicker(null); }} onClose={() => setPicker(null)} />
        )}
      </Show>
    </>
  );
}
