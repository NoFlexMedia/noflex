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
