// SPDX-License-Identifier: AGPL-3.0-only
//
// Combining lists from several servers. Scores aren't comparable across servers,
// so lists merge by rank, the same way hybrid search fuses its own.

const RRF_K = 60;

type ProviderIds = readonly { kind: string; value: string }[] | null | undefined;

const idKeys = (ids: ProviderIds): string[] => (ids ?? []).filter((id) => id.kind !== "CUSTOM").map((id) => `${id.kind}:${id.value}`);

/**
 * What makes two works the same title: a shared provider ID. Paths and other
 * server-local IDs never count, and unidentified works never merge, since a wrong
 * merge is worse than a duplicate. Episodes rarely have IDs of their own, so their
 * series' IDs and their numbers identify them too.
 */
function providerKeys(work: any): string[] {
  const own = idKeys(work.externalIds);
  if (work.parentIndex == null || work.indexNumber == null) return own;
  return [...own, ...idKeys(work.seriesExternalIds).map((key) => `${key}/S${work.parentIndex}E${work.indexNumber}`)];
}

/**
 * Reciprocal rank fusion of `lists`, best first. Works matching by `keys` become
 * one entry, which pools their scores and keeps the version from the earliest list
 * unless `prefer` picks a later one, so the home server wins a tie.
 */
export function mergeRanked<Work extends { id: string }>(
  lists: readonly (readonly Work[])[],
  keys: (work: Work) => string[] = providerKeys,
  prefer: (candidate: Work, current: Work) => boolean = () => false,
): Work[] {
  const entries: { work: Work; score: number; order: number }[] = [];
  const byKey = new Map<string, (typeof entries)[number]>();
  lists.forEach((list) => {
    list.forEach((work, rank) => {
      const workKeys = keys(work);
      const known = workKeys.map((key) => byKey.get(key)).find(Boolean);
      const score = 1 / (RRF_K + rank + 1);
      const entry = known ?? { work, score: 0, order: entries.length };
      if (!known) entries.push(entry);
      else if (prefer(work, known.work)) known.work = work;
      entry.score += score;
      for (const key of workKeys) byKey.set(key, entry);
    });
  });
  return entries.sort((a, b) => b.score - a.score || a.order - b.order).map((entry) => entry.work);
}

// Strongest first. A name the viewer typed outranks a word in a synopsis, which
// outranks a neighbour by meaning, however well that neighbour does on its own server.
const MATCH_STRENGTH = ["title", "credits", "overview", "meaning"];

const strongestFirst = (fields: readonly string[]): string[] =>
  [...new Set(fields)].sort((a, b) => MATCH_STRENGTH.indexOf(a) - MATCH_STRENGTH.indexOf(b));

interface SearchHit { work: { id: string }; matchedFields: string[]; similarity?: number | null }

/**
 * Search hits from several servers, best first. A title on two servers is no more
 * relevant for it, so unlike {@link mergeRanked} nothing pools: it keeps its best
 * showing. Names and synopsis matches go by each server's own rank; pure meaning
 * matches by similarity, which compares across servers on the same embedding model
 * and at worst shuffles the weakest results when they differ.
 */
export function mergeSearchHits<Hit extends SearchHit>(lists: readonly (readonly Hit[])[]): Hit[] {
  const entries: { hit: Hit; rank: number; order: number }[] = [];
  const byKey = new Map<string, (typeof entries)[number]>();
  lists.forEach((list) => {
    list.forEach((hit, rank) => {
      // The ID too: paging re-runs a search, so a shifted rank can repeat a hit.
      const keys = [hit.work.id, ...providerKeys(hit.work)];
      const known = keys.map((key) => byKey.get(key)).find(Boolean);
      const entry = known ?? { hit, rank, order: entries.length };
      if (!known) entries.push(entry);
      entry.rank = Math.min(entry.rank, rank);
      entry.hit = {
        ...entry.hit,
        matchedFields: strongestFirst([...entry.hit.matchedFields, ...hit.matchedFields]),
        similarity: Math.max(entry.hit.similarity ?? 0, hit.similarity ?? 0) || null,
      };
      for (const key of keys) byKey.set(key, entry);
    });
  });
  const strength = ({ hit }: (typeof entries)[number]) => MATCH_STRENGTH.indexOf(hit.matchedFields[0]!);
  return entries
    .sort((a, b) => strength(a) - strength(b)
      || (strength(a) === MATCH_STRENGTH.length - 1 ? (b.hit.similarity ?? 0) - (a.hit.similarity ?? 0) : 0)
      || a.rank - b.rank
      || a.order - b.order)
    .map((entry) => entry.hit);
}

const progress = (work: any): number[] => [work.parentIndex ?? 0, work.indexNumber ?? 0, work.userData?.positionMs ?? 0];

function furtherAlong(candidate: unknown, current: unknown): boolean {
  const [ours, theirs] = [progress(candidate), progress(current)];
  const differs = ours.findIndex((value, index) => value !== theirs[index]);
  return differs >= 0 && ours[differs]! > theirs[differs]!;
}

/**
 * Continue Watching, where a series is one entry however many servers have it.
 * Servers can disagree on the episode when sync is off or behind, and the one
 * further along is where the viewer actually is.
 */
export function mergeContinueWatching<Work extends { id: string }>(lists: readonly (readonly Work[])[]): Work[] {
  return mergeRanked(lists, (work: any) => [...idKeys(work.seriesExternalIds), ...providerKeys(work)], furtherAlong);
}
