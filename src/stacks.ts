import type { AutoScore } from "./autoScore";
import type { PhotoRecord } from "./types";

// What the grid actually draws: either a plain photo, or one member of a stack.
export interface GridItem {
  photo: PhotoRecord;
  stack: null | {
    id: string;
    // How many photos the stack holds among the currently visible ones.
    size: number;
    // The stack's best frame: the only one shown while the stack is collapsed.
    isCover: boolean;
    expanded: boolean;
  };
}

// Turns the (already filtered and sorted) photos into grid items, collapsing each series
// into one stack. Stacks only ever contain unrated photos with a series - the same set the
// auto-scoring covers - and they exist purely in the UI: nothing is written to any file.
export function buildGridItems(
  photos: PhotoRecord[],
  scores: Map<string, AutoScore>,
  enabled: boolean,
  expanded: Set<string>,
): GridItem[] {
  const seriesOf = (p: PhotoRecord): string | null =>
    enabled && !p.rated ? (scores.get(p.path)?.seriesId ?? null) : null;

  const members = new Map<string, PhotoRecord[]>();
  for (const photo of photos) {
    const id = seriesOf(photo);
    if (id) members.set(id, [...(members.get(id) ?? []), photo]);
  }

  const covers = new Map<string, string>();
  for (const [id, list] of members) {
    if (list.length < 2) continue;
    // The cover is the series' own best frame; if filters hide it, the best visible one.
    const best =
      list.find((p) => scores.get(p.path)?.seriesBest) ??
      list.reduce((a, b) =>
        (scores.get(b.path)?.score ?? 0) > (scores.get(a.path)?.score ?? 0) ? b : a,
      );
    covers.set(id, best.path);
  }

  const items: GridItem[] = [];
  const emitted = new Set<string>();
  for (const photo of photos) {
    const id = seriesOf(photo);
    const list = id ? members.get(id) : undefined;
    if (!id || !list || list.length < 2) {
      items.push({ photo, stack: null });
      continue;
    }
    if (emitted.has(id)) continue;
    emitted.add(id);

    const isOpen = expanded.has(id);
    const coverPath = covers.get(id);
    const shown = isOpen ? list : list.filter((p) => p.path === coverPath);
    for (const member of shown) {
      items.push({
        photo: member,
        stack: { id, size: list.length, isCover: member.path === coverPath, expanded: isOpen },
      });
    }
  }
  return items;
}
