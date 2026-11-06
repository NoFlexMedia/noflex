// SPDX-License-Identifier: AGPL-3.0-only
//
// Settings: a list of sections, the chosen one beside it. What follows the viewer is
// the `playback` preference every client shares; what belongs to this TV stays on it.
// Profile, sessions and server administration are easier with a real keyboard, and stay
// in the web client and the apps.

import { Column, Row } from "@solidtv/solid/primitives";
import { createEffect, createSignal, For, Show, untrack } from "solid-js";
import type { NodeProps } from "@solidtv/solid";
import { languageName } from "$lib/format.js";
import { MY_DEVICES, UPDATE_DEVICE } from "$lib/graphql/documents.js";
import { request } from "$lib/graphql.js";
import type { IconName } from "$lib/icons.generated.js";
import { app, changeServer, logout, savePlaybackPrefs, setShowLibraries, thisDeviceId } from "$lib/session.js";
import { changeSetting, settings, type DeviceSettings } from "$lib/settings.js";
import { serverOrigin } from "$lib/tokens.js";
import type { Choice } from "$lib/tracks.js";
import type { Device } from "$lib/types.js";
import { attempt } from "$lib/ui.js";
import { color, motion, PAGE_WIDTH, radius, SCREEN, type, weight } from "../theme.js";
import Button from "$components/Button.js";
import Icon from "$components/Icon.js";
import Page, { BODY_HEIGHT } from "$components/Page.js";
import SettingRow from "$components/SettingRow.js";
import Sheet from "$components/Sheet.js";

const SECTIONS_WIDTH = 340;
const GAP = 48;
const PANEL_WIDTH = PAGE_WIDTH - SECTIONS_WIDTH - GAP - 64;
const ROW_GAP = 10;
const GB = 1024 ** 3;

type Section = "playback" | "player" | "tv" | "downloads" | "account";
const SECTIONS: { id: Section; label: string; icon: IconName }[] = [
  { id: "playback", label: "Playback", icon: "play" },
  { id: "player", label: "Player", icon: "sliders-horizontal" },
  { id: "tv", label: "This TV", icon: "television" },
  { id: "downloads", label: "Smart Downloads", icon: "download-simple" },
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
const BUDGETS: Choice[] = [{ value: "", label: "No limit" }, ...[5, 10, 25, 50, 100].map((size) => ({ value: String(size * GB), label: `${size} GB` }))];
const SKIPS: Choice[] = [5, 10, 15, 30].map((seconds) => ({ value: String(seconds), label: `${seconds} seconds` }));
const COUNTDOWNS: Choice[] = [5, 10, 20, 30].map((seconds) => ({ value: String(seconds), label: `${seconds} seconds` }));
const CAPTION_SIZES: Choice[] = [{ value: "small", label: "Small" }, { value: "medium", label: "Medium" }, { value: "large", label: "Large" }];
const CAPTION_BACKS: Choice[] = [{ value: "none", label: "None" }, { value: "dim", label: "Dimmed" }, { value: "solid", label: "Solid" }];

const labelOf = (choices: Choice[], value: string) => choices.find((choice) => choice.value === value)?.label ?? value;
// A language picked here goes first; any others set from a browser stay behind it.
const putFirst = (list: string[], first: string) => (first ? [first, ...list.filter((item) => item !== first)] : []);

// Only the apps on phones, tablets and computers keep files: a browser or a TV has nowhere to.
const canKeepFiles = (device: Device) => !/\b(chrome|firefox|safari|edge|browser)\b|\btv\b|tvos/i.test(device.name);

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
  const [downloadsAllowed, setDownloadsAllowed] = createSignal(true);

  createEffect(() => {
    void request<{ myDevices: Device[]; serverInfo: { downloadsAllowed: boolean } }>(MY_DEVICES).then((data) => {
      setDevices(data.myDevices);
      setDownloadsAllowed(data.serverInfo.downloadsAllowed);
    }, () => setDevices([]));
  });

  const prefs = () => app.playbackPrefs;
  const here = () => devices()?.find((device) => device.id === thisDeviceId());
  const keepers = () => (devices() ?? []).filter((device) => device.id !== thisDeviceId() && canKeepFiles(device));

  const savePlayback = (change: (stored: any) => any) => attempt(() => savePlaybackPrefs(change));
  const saveDevice = (device: Device, change: Record<string, unknown>) => attempt(async () => {
    const data = await request<{ updateDevice: Device }>(UPDATE_DEVICE, { id: device.id, ...change });
    setDevices((list) => (list ?? []).map((item) => (item.id === device.id ? { ...item, ...data.updateDevice } : item)));
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
        <SettingRow width={PANEL_WIDTH} label="HDR" detail="Turn off if HDR looks wrong here; the server converts it to SDR" checked={settings.allowHdr} onEnter={() => { device("allowHdr", !settings.allowHdr); return true; }} />
        <Show when={here()} fallback={<Note text="This TV hasn’t registered with the server yet. It does when it starts." />}>
          {(me) => (
            <>
              <SettingRow width={PANEL_WIDTH} label="Prepare versions on the server" detail="While it’s idle, it encodes what you’ll likely watch for this TV" checked={me().smartTranscode} onEnter={() => { void saveDevice(me(), { smartTranscode: !me().smartTranscode }); return true; }} />
              <SettingRow width={PANEL_WIDTH} label="Quality of prepared versions" value={labelOf(PREPARE, String(me().maxBitrateBps ?? ""))} onEnter={() => choose("Quality of prepared versions", PREPARE, String(me().maxBitrateBps ?? ""), (value) => void saveDevice(me(), value ? { maxBitrateBps: Number(value) } : { clearMaxBitrate: true }))} />
            </>
          )}
        </Show>
      </>
    ),
    downloads: () => (
      <>
        <Note text="Keeps what you’re likely to watch on your phones, tablets and computers, ready for when you’re offline. They pick up a change here the next time the app starts. TVs stream, so this TV isn’t listed." />
        <Show when={!downloadsAllowed()}>
          <Note text="This server’s administrator has turned downloads off, so nothing new will be kept." />
        </Show>
        <Show when={devices() && !keepers().length}>
          <Heading text="No devices that can keep titles yet" />
          <Note text="Sign in with the NoFlex app on a phone, tablet or computer, and it shows up here." />
        </Show>
        <For each={keepers()}>
          {(item) => {
            const current = () => (devices() ?? []).find((known) => known.id === item.id) ?? item;
            return (
              <>
                <Heading text={item.name} />
                <SettingRow width={PANEL_WIDTH} label="Smart Downloads" checked={current().smartDownloads} onEnter={() => { void saveDevice(current(), { smartDownloads: !current().smartDownloads }); return true; }} />
                <SettingRow width={PANEL_WIDTH} label="Space to use" value={labelOf(BUDGETS, String(current().downloadBudgetBytes ?? ""))} onEnter={() => choose(`Space to use on ${item.name}`, BUDGETS, String(current().downloadBudgetBytes ?? ""), (value) => void saveDevice(current(), value ? { downloadBudgetBytes: Number(value) } : { clearDownloadBudget: true }))} />
                <SettingRow width={PANEL_WIDTH} label="Wait for Wi-Fi" checked={current().downloadsWifiOnly} onEnter={() => { void saveDevice(current(), { downloadsWifiOnly: !current().downloadsWifiOnly }); return true; }} />
              </>
            );
          }}
        </For>
      </>
    ),
    account: () => (
      <>
        <view skipFocus width={PANEL_WIDTH} height={150}>
          <view y={8} width={120} height={120} borderRadius={60} color={color.accent}>
            <text width={120} height={120} lineHeight={120} contain="both" textAlign="center" fontSize={type.title} fontWeight={weight.bold} color={color.accentInk}>{(app.viewer?.name ?? "").slice(0, 2).toUpperCase()}</text>
          </view>
          <text x={150} y={18} fontSize={type.title} fontWeight={weight.semibold}>{app.viewer?.name ?? ""}</text>
          <text x={150} y={76} fontSize={type.caption} color={color.text2}>{`${app.viewer?.isAdmin ? "Administrator" : "Member"} · ${app.server?.name ?? "NoFlex"} v${app.server?.version ?? ""}`}</text>
          <text x={150} y={110} fontSize={22} color={color.text3}>{serverOrigin() ?? ""}</text>
        </view>
        <SettingRow width={PANEL_WIDTH} label="Show libraries in the navigation" detail="Alongside Movies, TV Shows and the rest" checked={app.showLibraries} onEnter={() => { void attempt(() => setShowLibraries(!app.showLibraries)); return true; }} />
        <Row width={PANEL_WIDTH} height={84} gap={20} scroll="none">
          <Button size="lg" icon="sign-out" label="Sign out" onEnter={() => { void attempt(logout); return true; }} />
          <Button size="lg" icon="globe" label="Change server" onEnter={() => { changeServer(); return true; }} />
        </Row>
        <Note text="Your profile, devices, and server administration are in the web client and the apps." />
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
                <Column width={PANEL_WIDTH} height={SCREEN.height * 2} gap={ROW_GAP} scroll="auto">
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
