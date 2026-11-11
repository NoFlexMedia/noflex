// SPDX-License-Identifier: AGPL-3.0-only

export const REPO = "https://github.com/NoFlexMedia/noflex";
export const blob = (path: string) => `${REPO}/blob/main/${path}`;

export const links = {
  repo: REPO,
  issues: `${REPO}/issues`,
  newIssue: `${REPO}/issues/new/choose`,
  discussions: `${REPO}/discussions`,
  advisories: `${REPO}/security/advisories/new`,
  releases: `${REPO}/releases`,
  gettingStarted: blob("docs/GETTING_STARTED.md"),
  configuration: blob("docs/CONFIGURATION.md"),
  deployment: blob("docs/DEPLOYMENT.md"),
  plugins: blob("docs/PLUGINS.md"),
  pluginLicensing: blob("docs/PLUGIN_LICENSING.md"),
  contributing: blob("CONTRIBUTING.md"),
  cla: blob("CLA.md"),
  license: blob("LICENSE"),
  blender: "https://studio.blender.org/films/",
};

export type Art = { slug: string; title: string; year: number; kind: string; runtime: string; ambient: string; overview: string; still?: false };

/** The demo library: Blender Studio's open movies (CC BY), with tints sampled from their backdrops. */
export const art: Art[] = [
  { slug: "sintel", title: "Sintel", year: 2010, kind: "Movie", runtime: "15m", ambient: "105 66 41", overview: "A lonely young woman raises a baby dragon she names Scales. When a larger dragon snatches him away, she crosses a frozen, hostile world to bring him home." },
  { slug: "tears-of-steel", title: "Tears of Steel", year: 2012, kind: "Movie", runtime: "12m", ambient: "136 140 109", still: false, overview: "In a future Amsterdam, a group of warriors and scientists replays a breakup to save the world from robots." },
  { slug: "big-buck-bunny", title: "Big Buck Bunny", year: 2008, kind: "Movie", runtime: "11m", ambient: "129 174 176", overview: "A gentle giant of a rabbit has his quiet morning ruined by three bullying rodents, and sets about teaching them a lesson." },
  { slug: "sprite-fright", title: "Sprite Fright", year: 2021, kind: "Movie", runtime: "10m", ambient: "77 57 34", overview: "A group of rowdy teenagers on a woodland trip find out that not all mushrooms are fun guys." },
  { slug: "charge", title: "Charge", year: 2022, kind: "Movie", runtime: "4m", ambient: "48 62 48", overview: "In a dystopian future, a lone old man breaks into an energy plant to steal the last charge he needs." },
  { slug: "caminandes", title: "Caminandes", year: 2013, kind: "Series", runtime: "3 episodes", ambient: "184 113 80", overview: "Koro the llama keeps finding a way past every fence in Patagonia." },
  { slug: "elephants-dream", title: "Elephants Dream", year: 2006, kind: "Movie", runtime: "11m", ambient: "55 57 53", overview: "Two strange characters explore a capricious and seemingly infinite machine." },
];

export const poster = (slug: string) => `/art/${slug}-poster.webp`;
export const backdrop = (slug: string) => `/art/${slug}-backdrop.webp`;
/** Titles whose stills the site may show: Tears of Steel's footage may not be used to advertise. */
export const stills = art.filter((item) => item.still !== false);

export const SITE = "https://noflex.dev";
/** The public demo server, once there is one: until then `/demo` says it's coming. */
export const DEMO_URL: string | null = null;
