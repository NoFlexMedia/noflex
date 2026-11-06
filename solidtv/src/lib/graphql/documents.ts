// SPDX-License-Identifier: AGPL-3.0-only
//
// Every operation the TV sends. `graphql.test.ts` validates them against the
// server's committed SDL snapshot, as Houdini and Ferry do for the other clients.

import { gql } from "$lib/graphql.js";

const WORK_CARD = `
fragment WorkCard on Work {
  id
  kind
  parentId
  libraryId
  title
  year
  communityRating
  indexNumber
  indexEnd
  parentIndex
  runtimeMs
  indexing
  images { kind url }
  userData { played positionMs favorite }
}`;

const EDITION = `
fragment EditionFull on Edition {
  id
  workId
  name
  runtimeMs
  introStartMs
  introEndMs
  creditsStartMs
  chapters { positionMs title }
  trickplay { intervalMs thumbWidth thumbHeight tileColumns tileRows sheets { index tileCount url } chapterCues { positionMs title } }
}`;

const MEDIA_FILE = `
fragment MediaFileFull on MediaFile {
  id
  editionId
  path
  partIndex
  sizeBytes
  container
  probe
  subtitleTracks { language codec default forced title srtCompatible }
}`;

const USER_DATA = `
fragment UserDataFields on UserData { played positionMs favorite }`;

const SESSION_FIELDS = `
fragment SessionFields on PlaybackSession {
  sessionId
  route
  url
  jobId
  startPositionMs
  subtitles { index language title forced default url }
  decision {
    plan
    reasons { code description }
    source { ...StreamFields }
    target { ...StreamFields }
    encoder
    audioEncoder
    hardware
    tonemap
  }
}
fragment StreamFields on StreamSummary { container videoCodec videoProfile videoLevel bitDepth width height videoRange audioCodec audioChannels bitrateBps }`;

// ---- session ----------------------------------------------------------------

export const SERVER_INFO = `query ServerInfo { serverInfo { name version backend isConfigured } }`;

export const BOOTSTRAP = `
query Bootstrap {
  viewer { id name username avatarUrl isAdmin }
  libraries { id name kind }
  contentCategories { category workCount }
  preferences { key value }
  serverInfo { name version backend isConfigured }
}`;

export const LOGIN = `
mutation Login($username: String!, $password: String!, $client: String!, $device: String!) {
  login(username: $username, password: $password, client: $client, device: $device) { token refreshToken }
}`;

export const LOGOUT = `mutation Logout { logout }`;

export const CREATE_DEVICE_PIN = `
mutation CreateDevicePin($client: String!, $device: String!) {
  createDevicePin(client: $client, device: $device) { pin expiresAt }
}`;

export const POLL_DEVICE_PIN = `mutation PollDevicePin($pin: String!) { pollDevicePin(pin: $pin) { token refreshToken } }`;

export const SET_PREFERENCE = `mutation SetPreference($key: String!, $value: JSON!) { setPreference(key: $key, value: $value) { key } }`;

const DEVICE = `
fragment DeviceFields on Device { id name smartTranscode maxBitrateBps }`;

export const MY_DEVICES = gql(`query MyDevices { myDevices { ...DeviceFields } }`, DEVICE);

const SMART_MARK = `
fragment SmartMarkFields on SmartTranscodeMark {
  work { ...WorkCard }
  device { id name }
  markedAt
  state
  progress
}`;

export const WORKS_BY_ID = gql(`query WorksById($ids: [ID!]!) { worksById(ids: $ids) { ...WorkCard } }`, WORK_CARD);

export const MY_SMART_MARKS = gql(`query MySmartMarks { mySmartTranscodeMarks { ...SmartMarkFields } }`, SMART_MARK, WORK_CARD);

export const PREPARED_VERSIONS = `query PreparedVersions { preparedVersions { id description sizeBytes lastUsedAt work { id title } } }`;

export const SMART_MARKS_FOR_WORK = `query SmartMarksForWork($work: ID!) { mySmartTranscodeMarks(workId: $work) { device { id } state progress } myDevices { smartTranscode } }`;

export const MARK_FOR_SMART_TRANSCODING = `mutation MarkForSmartTranscoding($work: ID!, $device: ID) { markForSmartTranscoding(workId: $work, deviceId: $device) { state } }`;

export const UNMARK_FOR_SMART_TRANSCODING = `mutation UnmarkForSmartTranscoding($work: ID!, $device: ID) { unmarkForSmartTranscoding(workId: $work, deviceId: $device) { markedAt } }`;

export const UPDATE_DEVICE = gql(`
mutation UpdateDevice($id: ID!, $smartTranscode: Boolean, $maxBitrateBps: Int, $clearMaxBitrate: Boolean) {
  updateDevice(id: $id, smartTranscode: $smartTranscode, maxBitrateBps: $maxBitrateBps, clearMaxBitrate: $clearMaxBitrate) { ...DeviceFields }
}`, DEVICE);

export const REGISTER_DEVICE = `
mutation RegisterDevice($id: ID!, $name: String!, $capabilities: ClientCapabilities!) {
  registerDevice(id: $id, name: $name, capabilities: $capabilities) { id }
}`;

// ---- browse -----------------------------------------------------------------

export const HOME = gql(`
query Home {
  contentCategories { category workCount }
  continueWatching(first: 16) { nodes { ...WorkCard seriesTitle } }
  recentlyAdded(first: 24) { nodes { ...WorkCard overview tagline } }
  recentlyPlayed(first: 16) { nodes { ...WorkCard seriesTitle } }
  recommendations(first: 16) { nodes { ...WorkCard } }
}`, WORK_CARD);

export const CATEGORY_SHELVES = gql(`
query CategoryShelves($category: ContentCategory!) {
  recommendations(category: $category, first: 24) { nodes { ...WorkCard } }
  newlyReleased: works(category: $category, sort: DATE_RELEASED_DESC, first: 24) { nodes { ...WorkCard } }
  recentlyAdded: works(category: $category, sort: DATE_ADDED_DESC, first: 24) { nodes { ...WorkCard } }
}`, WORK_CARD);

export const CATEGORY_WORKS = gql(`
query CategoryWorks($category: ContentCategory!, $after: String) {
  works(category: $category, sort: TITLE_ASC, first: 48, after: $after) {
    nodes { ...WorkCard }
    pageInfo { hasNextPage endCursor }
  }
}`, WORK_CARD);

export const LIBRARY_WORKS = gql(`
query LibraryWorks($library: ID!, $after: String) {
  works(libraryId: $library, sort: TITLE_ASC, first: 48, after: $after) {
    nodes { ...WorkCard }
    pageInfo { hasNextPage endCursor }
  }
}`, WORK_CARD);

export const WORK_DETAIL = gql(`
query WorkDetail($id: ID!) {
  work(id: $id) {
    ...WorkCard
    overview
    tagline
    credits { personId name role character }
    tags(namespaces: ["parental", "genre"]) { id namespace display }
  }
  editions(workId: $id) { ...EditionFull }
}`, WORK_CARD, EDITION);

export const WORK_CRUMB = gql(`query WorkCrumb($id: ID!) { work(id: $id) { ...WorkCard } }`, WORK_CARD);

export const MEDIA_FILES = gql(`query MediaFiles($edition: ID!) { mediaFiles(editionId: $edition) { ...MediaFileFull } }`, MEDIA_FILE);

export const CHILD_PAGE = gql(`
query ChildPage($library: ID!, $parent: ID!, $after: String) {
  works(libraryId: $library, filter: { parentId: $parent, recursive: false }, sort: TITLE_ASC, first: 60, after: $after) {
    nodes { ...WorkCard overview }
    pageInfo { hasNextPage endCursor }
  }
}`, WORK_CARD);

export const SEARCH = gql(`
query Search($text: String!, $mode: SearchMode!) {
  search(text: $text, mode: $mode, first: 48) {
    edges { node { id work { ...WorkCard seriesTitle } } }
  }
}`, WORK_CARD);

export const COLLECTIONS = gql(`
query Collections($library: ID!, $after: String) {
  collections(libraryId: $library, first: 16, after: $after) {
    nodes { id title items(first: 12) { nodes { ...WorkCard } } }
    pageInfo { hasNextPage endCursor }
  }
}`, WORK_CARD);

// ---- playback ---------------------------------------------------------------

export const START_PLAYBACK = gql(`
mutation StartPlayback($media: ID!, $capabilities: ClientCapabilities!, $subtitle: Int, $start: Int, $audio: Int) {
  startPlaybackSession(mediaFileId: $media, capabilities: $capabilities, burnSubtitle: $subtitle, startPositionMs: $start, audioStream: $audio) { ...SessionFields }
}`, SESSION_FIELDS);

export const STOP_PLAYBACK = `mutation StopPlayback($session: ID!) { stopPlaybackSession(sessionId: $session) }`;

export const REPORT_PROGRESS = gql(`
mutation ReportProgress($id: ID!, $position: Int!, $state: PlaybackState!) {
  reportProgress(target: { kind: WORK, id: $id }, positionMs: $position, state: $state) { ...UserDataFields }
}`, USER_DATA);

export const SET_FAVORITE = gql(`
mutation SetFavorite($id: ID!, $favorite: Boolean!) {
  setFavorite(target: { kind: WORK, id: $id }, favorite: $favorite) { ...UserDataFields }
}`, USER_DATA);

export const MARK_PLAYED = gql(`mutation MarkPlayed($id: ID!) { markPlayed(target: { kind: WORK, id: $id }) { ...UserDataFields } }`, USER_DATA);
export const MARK_UNPLAYED = gql(`mutation MarkUnplayed($id: ID!) { markUnplayed(target: { kind: WORK, id: $id }) { ...UserDataFields } }`, USER_DATA);

// ---- live TV ----------------------------------------------------------------

export const LIVE_CHANNELS = `
query LiveChannels {
  liveTvChannels(first: 100) { nodes { id name number logoUrl group enabled } }
}`;

export const LIVE_GUIDE = `
query LiveGuide($channelIds: [ID!]!, $start: DateTime!, $end: DateTime!) {
  liveTvGuide(channelIds: $channelIds, start: $start, end: $end, first: 400) {
    nodes { id channelId title startAt endAt }
  }
}`;

export const START_LIVE = gql(`
mutation StartLiveChannel($id: ID!) { startLiveTvChannel(channelId: $id) { ...SessionFields } }`, SESSION_FIELDS);

// ---- server status, for administrators --------------------------------------

export const SERVER_ACTIVITY = `
query ServerActivity {
  systemStatus { activeScans plugins { circuitOpen } server { database { healthy } } }
  activeStreams { id transcode { status } }
  optimizedVersionStats { running }
}`;
