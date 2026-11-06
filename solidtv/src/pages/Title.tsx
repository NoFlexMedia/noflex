// SPDX-License-Identifier: AGPL-3.0-only
//
// A title's page: what it is, how to play it, and what's inside it. Everything an
// administrator would change about it stays in the web client.

import { useNavigate, useParams } from "@solidjs/router";
import { Column, Row } from "@solidtv/solid/primitives";
import { createEffect, createMemo, createSignal, on, onCleanup, Show, untrack } from "solid-js";
import { audioFormat, runtime, titleCase, versionLabel } from "$lib/format.js";
import { MARK_PLAYED, MARK_UNPLAYED, MEDIA_FILES, SET_FAVORITE, WORK_CRUMB, WORK_DETAIL } from "$lib/graphql/documents.js";
import { createQuery, request } from "$lib/graphql.js";
import { pickImage } from "$lib/images.js";
import { fetchChildren, isPlayable, player, playWork, subtitleFor } from "$lib/playback.js";
import { app } from "$lib/session.js";
import { audioChoices, preferredAudio, preferredSubtitle, subtitleChoices } from "$lib/tracks.js";
import type { Edition, MediaFile, Work } from "$lib/types.js";
import { attempt, ui } from "$lib/ui.js";
import { setUserData, userDataOf } from "$lib/watch.js";
import { color, PAGE_WIDTH, PAGE_X, SCREEN, type, weight } from "../theme.js";
import Button from "$components/Button.js";
import Icon from "$components/Icon.js";
import Rail, { railHeight } from "$components/Rail.js";
import Sheet from "$components/Sheet.js";
import Skeleton from "$components/Skeleton.js";
import State from "$components/State.js";
import Artwork from "$components/Artwork.js";

const HERO_HEIGHT = 840;
type SheetKind = "version" | "edition" | "audio" | "subtitle";

function Children(props: { work: Work; onUpNext: (next: { work: Work; resume?: boolean; rewatch?: boolean } | null) => void }) {
  const navigate = useNavigate();
  const [top, setTop] = createSignal<Work[]>([]);
  const [seasonId, setSeasonId] = createSignal<string | null>(null);
  const [episodes, setEpisodes] = createSignal<Work[]>([]);

  createEffect(on(() => props.work.id, () => {
    let stale = false;
    onCleanup(() => { stale = true; });
    (async () => {
      const children = await fetchChildren(props.work.libraryId, props.work.id);
      if (stale) return;
      setTop(children);
      const seasons = children.filter((child) => child.kind === "SEASON");
      setSeasonId(seasons[0]?.id ?? null);
      if (props.work.kind !== "SERIES" && props.work.kind !== "SEASON") return;
      // The episode to start from: in progress first, then the first unwatched, else the very first.
      const groups = seasons.length ? await Promise.all(seasons.map((season) => fetchChildren(props.work.libraryId, season.id))) : [children];
      if (stale) return;
      const all = groups.reduce((sum, group) => sum.concat(group), [] as Work[]);
      const started = all.find((item) => !item.userData?.played && (item.userData?.positionMs ?? 0) > 0);
      const next = started ?? all.find((item) => !item.userData?.played);
      props.onUpNext(next ? { work: next, resume: !!started } : all[0] ? { work: all[0], rewatch: true } : null);
    })().catch(() => {});
  }));

  createEffect(on(seasonId, (id) => {
    if (!id) { setEpisodes([]); return; }
    let stale = false;
    onCleanup(() => { stale = true; });
    fetchChildren(props.work.libraryId, id).then((list) => { if (!stale) setEpisodes(list); }).catch(() => {});
  }));

  const seasons = () => top().filter((child) => child.kind === "SEASON");
  const flat = () => top().filter((child) => child.kind !== "SEASON");
  const flatIsEpisodes = () => flat().some((child) => child.kind === "EPISODE" || child.kind === "TRACK");
  const flatTitle = () => (props.work.kind === "ARTIST" ? "Albums" : props.work.kind === "ALBUM" ? "Tracks" : "Episodes");
  const pickSeason = (season: Work) => {
    if (season.id === seasonId()) navigate(`/works/${season.id}`);
    else setSeasonId(season.id);
  };

  return (
    <>
      <Show when={seasons().length}>
        <Rail title="Seasons" items={seasons()} height={railHeight()} onChosen={pickSeason} />
      </Show>
      <Show when={episodes().length}>
        <Rail title={seasons().find((season) => season.id === seasonId())?.title ?? "Episodes"} items={episodes()} wide height={railHeight(true)} />
      </Show>
      <Show when={flat().length}>
        <Rail title={flatTitle()} items={flat()} wide={flatIsEpisodes()} height={railHeight(flatIsEpisodes())} />
      </Show>
    </>
  );
}

export default function Title() {
  const params = useParams();
  const detail = createQuery<{ work: Work; editions: Edition[] }>(WORK_DETAIL, () => ({ id: params.id }));
  const work = () => detail.data()?.work;
  const editions = () => detail.data()?.editions ?? [];

  const [editionId, setEditionId] = createSignal<string | null>(null);
  createEffect(() => { if (!editions().some((edition) => edition.id === editionId())) setEditionId(editions()[0]?.id ?? null); });
  const edition = () => editions().find((item) => item.id === editionId());

  const media = createQuery<{ mediaFiles: MediaFile[] }>(MEDIA_FILES, () => (editionId() ? { edition: editionId() } : null));
  const files = () => media.data()?.mediaFiles ?? [];
  const [mediaId, setMediaId] = createSignal<string | null>(null);
  createEffect(() => { if (!files().some((file) => file.id === mediaId())) setMediaId(files()[0]?.id ?? null); });
  const file = () => files().find((item) => item.id === mediaId());

  const audioStreams = () => file()?.probe?.audio ?? [];
  const subtitleTracks = () => file()?.subtitleTracks ?? [];
  const [audio, setAudio] = createSignal<number | null>(null);
  const [subtitle, setSubtitle] = createSignal<number | null>(null);
  createEffect(on(mediaId, () => {
    setAudio(preferredAudio(audioStreams(), app.playbackPrefs));
    setSubtitle(preferredSubtitle(subtitleTracks(), app.playbackPrefs));
  }));

  const [upNext, setUpNext] = createSignal<{ work: Work; resume?: boolean; rewatch?: boolean } | null>(null);
  createEffect(on(() => params.id, () => setUpNext(null)));
  const [sheet, setSheet] = createSignal<SheetKind | null>(null);

  // A season or episode without art of its own wears its series'.
  const parent = createQuery<{ work: Work }>(WORK_CRUMB, () => (work()?.parentId ? { id: work()!.parentId } : null));
  const heroArt = () => pickImage(work()?.images, "backdrop") ?? pickImage(parent.data()?.work.images, "backdrop");
  createEffect(() => { ui.backdrop = heroArt(); });

  const known = createMemo(() => (work() ? userDataOf(work()!) : null));
  const resumeMs = () => (known() && !known()!.played ? known()!.positionMs ?? 0 : 0);
  const canPlay = () => isPlayable(work());
  const genres = () => work()?.tags?.filter((tag) => tag.namespace === "genre").map((tag) => tag.display).slice(0, 3) ?? [];
  const parental = () => work()?.tags?.find((tag) => tag.namespace === "parental")?.display;
  const meta = () => [subtitleFor(work()) || work()?.year, titleCase(work()?.kind ?? ""), runtime(edition()?.runtimeMs ?? work()?.runtimeMs), parental(), ...genres()].filter(Boolean).join("  ·  ");
  const tech = () => [versionLabel(file()?.probe), audioStreams()[audio() ?? 0] ? audioFormat(audioStreams()[audio() ?? 0]) : ""].filter(Boolean).join("  ·  ");
  const cast = () => (work()?.credits ?? []).filter((credit) => credit.role === "ACTOR" || credit.character).slice(0, 5).map((credit) => credit.name).join(", ");
  const directedBy = () => (work()?.credits ?? []).filter((credit) => credit.role === "DIRECTOR").slice(0, 2).map((credit) => credit.name).join(", ");

  const options = () => ({ editionId: editionId() ?? undefined, mediaFileId: mediaId() ?? undefined, audioStream: audio(), subtitleIndex: subtitle() });
  const start = (fromStart: boolean) => {
    const current = work();
    if (current) void playWork(current, { ...options(), fromStart });
    return true;
  };

  const toggleFavorite = () => {
    const current = work();
    if (!current) return true;
    void attempt(async () => {
      const data = await request(SET_FAVORITE, { id: current.id, favorite: !known()?.favorite });
      setUserData(current.id, data.setFavorite);
    });
    return true;
  };
  const toggleWatched = () => {
    const current = work();
    if (!current) return true;
    void attempt(async () => {
      if (known()?.played) setUserData(current.id, (await request(MARK_UNPLAYED, { id: current.id })).markUnplayed);
      else setUserData(current.id, (await request(MARK_PLAYED, { id: current.id })).markPlayed);
    });
    return true;
  };

  // Read once when opened: the values it picks would otherwise rebuild it mid-pick.
  const sheetFor = (kind: SheetKind) => {
    switch (kind) {
      case "edition": return { title: "Edition", choices: editions().map((item) => ({ value: item.id, label: item.name || "Standard" })), value: editionId() ?? "", pick: (value: string) => setEditionId(value) };
      case "version": return { title: "Version", choices: files().map((item) => ({ value: item.id, label: versionLabel(item.probe) ?? item.path.split("/").pop() ?? "File" })), value: mediaId() ?? "", pick: (value: string) => setMediaId(value) };
      case "audio": return { title: "Audio", choices: audioChoices(audioStreams()), value: String(audio() ?? 0), pick: (value: string) => setAudio(Number(value)) };
      case "subtitle": return { title: "Subtitles", choices: subtitleChoices(subtitleTracks()), value: subtitle() == null ? "off" : String(subtitle()), pick: (value: string) => setSubtitle(value === "off" ? null : Number(value)) };
    }
  };

  return (
    <view x={PAGE_X} width={PAGE_WIDTH} height={SCREEN.height} forwardFocus={0}>
      <Show when={work()} keyed fallback={
        <Show when={detail.error()} fallback={<Skeleton y={140} />}>
          <State areaWidth={PAGE_WIDTH} autofocus kind="error" title="This title is out of reach" message={detail.error()} action={() => void detail.refetch()} />
        </Show>
      }>
        {(current) => (
          <>
            <Column autofocus width={PAGE_WIDTH} height={SCREEN.height * 2} scroll="always" gap={8}>
              <view width={PAGE_WIDTH} height={HERO_HEIGHT} forwardFocus={0}>
                <Row y={588} width={PAGE_WIDTH - 80} height={80} gap={18} scroll="auto">
                  <Show when={canPlay()}>
                    <Button
                      size="lg"
                      kind="light"
                      icon="play"
                      busy={player.preparing === current.id}
                      label={resumeMs() > 60_000 ? `Resume · ${runtime((edition()?.runtimeMs ?? current.runtimeMs ?? 0) - resumeMs())} left` : "Play"}
                      onEnter={() => start(false)}
                    />
                    <Show when={resumeMs() > 60_000}>
                      <Button size="lg" icon="clock-counter-clockwise" label="From the start" onEnter={() => start(true)} />
                    </Show>
                  </Show>
                  {/* There from the start, so focus doesn't jump when the next episode is worked out. */}
                  <Show when={current.kind === "SERIES" || current.kind === "SEASON"}>
                    <Button
                      size="lg"
                      kind="light"
                      icon="play"
                      width={340}
                      busy={!upNext()}
                      label={upNext() ? `${upNext()!.resume ? "Resume" : upNext()!.rewatch ? "Watch again" : "Play"} ${subtitleFor(upNext()!.work)}` : "Play"}
                      onEnter={() => { const next = upNext(); if (next) void playWork(next.work); return true; }}
                    />
                  </Show>
                  <Button size="lg" icon={known()?.favorite ? "heart-fill" : "heart"} onEnter={toggleFavorite} />
                  <Button size="lg" icon={known()?.played ? "check-circle" : "check"} onEnter={toggleWatched} />
                  <Show when={editions().length > 1}>
                    <Button size="lg" icon="stack" label={edition()?.name || "Edition"} onEnter={() => { setSheet("edition"); return true; }} />
                  </Show>
                  <Show when={canPlay() && files().length > 1}>
                    <Button size="lg" icon="film-strip" label="Version" onEnter={() => { setSheet("version"); return true; }} />
                  </Show>
                  <Show when={canPlay() && audioStreams().length > 1}>
                    <Button size="lg" icon="translate" label="Audio" onEnter={() => { setSheet("audio"); return true; }} />
                  </Show>
                  <Show when={canPlay() && subtitleTracks().length > 0}>
                    <Button size="lg" icon="subtitles" label="Subtitles" onEnter={() => { setSheet("subtitle"); return true; }} />
                  </Show>
                </Row>
                <view y={90} width={1300} height={474} display="flex" flexDirection="column" justifyContent="flexEnd" gap={18} skipFocus>
                  <text width={1300} contain="width" maxLines={2} lineHeight={84} fontSize={72} fontWeight={weight.bold}>{current.title}</text>
                  <view width={1300} height={36} display="flex" gap={14} alignItems="center">
                    <Show when={current.communityRating}>
                      <Icon name="star" size={28} color={color.warning} />
                      <text fontSize={type.caption} fontWeight={weight.semibold} color={color.warning}>{current.communityRating?.toFixed(1)}</text>
                    </Show>
                    <text fontSize={type.caption} fontWeight={weight.semibold} color={color.text2}>{meta()}</text>
                  </view>
                  <text width={1100} contain="width" maxLines={4} lineHeight={42} fontSize={type.body} color={color.text2}>{current.overview ?? current.tagline ?? ""}</text>
                </view>
                <text y={700} width={1400} contain="width" maxLines={1} fontSize={type.caption} color={color.text3}>{tech()}</text>
                <text y={742} width={1400} contain="width" maxLines={1} fontSize={type.caption} color={color.text3}>{[directedBy() && `Directed by ${directedBy()}`, cast() && `With ${cast()}`].filter(Boolean).join("   ·   ")}</text>
              </view>
              <Children work={current} onUpNext={setUpNext} />
            </Column>
            <Show when={current.kind === "PHOTO"}>
              <Artwork work={current} kind="poster" x={PAGE_WIDTH - 900} y={140} width={820} height={800} skipFocus />
            </Show>
          </>
        )}
      </Show>
      <Show when={sheet()} keyed>
        {(kind) => {
          const open = untrack(() => sheetFor(kind));
          return (
            <view x={-PAGE_X} width={SCREEN.width} height={SCREEN.height} forwardFocus={0}>
              <Sheet title={open.title} choices={open.choices} value={open.value} onPick={(value) => { open.pick(value); setSheet(null); }} onClose={() => setSheet(null)} />
            </view>
          );
        }}
      </Show>
    </view>
  );
}

