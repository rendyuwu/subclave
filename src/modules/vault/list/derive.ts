// Pure derivations over the vault list: which rows a scope shows, the counts
// the group tree and the tag strip draw, and whether an entry has expired.
//
// No React, no state library and no Tauri bindings, so a node script can load
// this file and check the rules that are only checkable while they are
// separable from the rendering.

import type { EntrySummary, Group } from "../types";

/** Pseudo-scope: every entry outside Trash. */
export const ALL_SCOPE = "all";
/** Pseudo-scope: every favourite outside Trash. */
export const FAVORITES_SCOPE = "favorites";
/** The Trash group, which is also a real group id. */
export const TRASH_SCOPE = "trash";

export function isExpired(entry: EntrySummary, now: number): boolean {
  return entry.expiresAt !== null && entry.expiresAt <= now;
}

/**
 * The rows a scope, the tag filter and the active query leave visible, sorted
 * by title with numeric collation (so "Item 2" precedes "Item 10"). The tag
 * filter is AND over lowercased keys; `searchIds` is the id intersection from
 * `vault_search` (`null` = no active query).
 */
export function visibleEntries(input: {
  entries: EntrySummary[];
  scope: string;
  tagFilter: string[];
  searchIds: readonly string[] | null;
  now: number;
}): EntrySummary[] {
  const { entries, scope, tagFilter, searchIds } = input;
  const ids = searchIds === null ? null : new Set(searchIds);
  const rows = entries.filter((entry) => {
    if (ids !== null && !ids.has(entry.id)) return false;
    if (tagFilter.length > 0) {
      const tags = entry.tags.map((tag) => tag.toLowerCase());
      if (!tagFilter.every((key) => tags.includes(key))) return false;
    }
    if (scope === ALL_SCOPE) return entry.groupId !== TRASH_SCOPE;
    if (scope === FAVORITES_SCOPE) return entry.favorite && entry.groupId !== TRASH_SCOPE;
    return entry.groupId === scope;
  });
  return rows.sort((a, b) => a.title.localeCompare(b.title, undefined, { numeric: true }));
}

/**
 * Direct-entry counts keyed by scope and by group id, seeded with a zero for
 * every group so an empty group renders its own count rather than a caller's
 * fallback. Descendants are not summed in.
 */
export function groupCounts(entries: EntrySummary[], groups: Group[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const group of groups) counts.set(group.id, 0);
  counts.set(ALL_SCOPE, 0);
  counts.set(FAVORITES_SCOPE, 0);
  counts.set(TRASH_SCOPE, 0);
  for (const entry of entries) {
    if (entry.groupId === TRASH_SCOPE) {
      counts.set(TRASH_SCOPE, (counts.get(TRASH_SCOPE) ?? 0) + 1);
      continue;
    }
    counts.set(ALL_SCOPE, (counts.get(ALL_SCOPE) ?? 0) + 1);
    if (entry.favorite) counts.set(FAVORITES_SCOPE, (counts.get(FAVORITES_SCOPE) ?? 0) + 1);
    counts.set(entry.groupId, (counts.get(entry.groupId) ?? 0) + 1);
  }
  return counts;
}

/**
 * Every tag in use on a live entry, one row per canonical (first-seen)
 * spelling, case-insensitively, sorted by name. Trashed entries are left out:
 * their tags are not offered as a filter for the live list.
 */
export function tagCounts(entries: EntrySummary[]): { tag: string; count: number }[] {
  const byKey = new Map<string, { tag: string; count: number }>();
  for (const entry of entries) {
    if (entry.groupId === TRASH_SCOPE) continue;
    for (const tag of entry.tags) {
      const key = tag.toLowerCase();
      const seen = byKey.get(key);
      if (seen) seen.count += 1;
      else byKey.set(key, { tag, count: 1 });
    }
  }
  return [...byKey.values()].sort((a, b) => a.tag.toLowerCase().localeCompare(b.tag.toLowerCase()));
}
