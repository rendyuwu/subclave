import { create } from "zustand";
import { subscribeVaultEvents } from "./events";
import * as vault from "./ipc";
import { ALL_SCOPE, visibleEntries } from "./list/derive";
import type { EntryDetail, EntrySummary, Group, VaultStatus } from "./types";

export type EditorRequest = { entryId: string | null; groupId: string };

/** State only, no actions: the payload itself lives in Rust and is never
 *  mirrored here, so nothing in this module needs a secret. */
export type VaultState = {
  /** `null` until the first status probe answers. */
  status: VaultStatus | null;
  entries: EntrySummary[];
  groups: Group[];
  /** ALL_SCOPE | FAVORITES_SCOPE | a group id. */
  scope: string;
  selectedId: string | null;
  detail: EntryDetail | null;
  query: string;
  /** `null` = no active query. */
  searchIds: string[] | null;
  /** Lowercased tag keys. */
  tagFilter: string[];
  /** Expanded group ids. */
  expanded: string[];
  /** From `subclave:vault-save-failed`. */
  saveError: string | null;
  editor: EditorRequest | null;
  /** From `subclave:quit-requested`. */
  quitPrompt: boolean;
};

type VaultActions = {
  init: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  refresh: () => Promise<void>;
  refreshDetail: () => Promise<void>;
  unlock: (masterPassword: string) => Promise<void>;
  create: (masterPassword: string) => Promise<void>;
  lock: () => Promise<void>;
  selectScope: (scope: string) => void;
  selectEntry: (id: string) => void;
  /**
   * Drop a selection the current scope, tag filter or query no longer shows.
   * Without it the detail pane keeps a hidden row and the list-scoped key
   * commands (Enter, Delete, Mod+C) act on it while the list shows something
   * else entirely.
   */
  pruneSelection: () => void;
  setQuery: (query: string) => Promise<void>;
  toggleTag: (key: string) => void;
  clearTags: () => void;
  toggleExpanded: (id: string) => void;
  openEditor: (entryId: string | null, groupId: string) => void;
  closeEditor: () => void;
  dismissQuit: () => void;
  retrySave: () => Promise<void>;
  restoreSnapshot: () => Promise<void>;
  touchActivity: () => void;
};

export type VaultStore = VaultState & VaultActions;

const STATUS_POLL_MS = 5_000;
const TOUCH_INTERVAL_MS = 30_000;

let initialized = false;
let lastTouchAt = 0;

export const useVaultStore = create<VaultStore>((set, get) => ({
  status: null,
  entries: [],
  groups: [],
  scope: ALL_SCOPE,
  selectedId: null,
  detail: null,
  query: "",
  searchIds: null,
  tagFilter: [],
  expanded: [],
  saveError: null,
  editor: null,
  quitPrompt: false,

  init: async () => {
    if (initialized) return;
    initialized = true;
    await subscribeVaultEvents({
      onLocked: () => {
        // Reset on lock: the unlocked vault must not come back to the group,
        // query or selection someone was looking at before it locked.
        set({
          entries: [],
          groups: [],
          detail: null,
          selectedId: null,
          editor: null,
          query: "",
          searchIds: null,
          tagFilter: [],
          scope: ALL_SCOPE,
          expanded: [],
        });
        void get().refreshStatus();
      },
      onChanged: () => {
        void get().refresh();
      },
      onSaveFailed: (reason) => set({ saveError: reason }),
      onQuitRequested: () => set({ quitPrompt: true }),
    });
    await get().refreshStatus();
    if (!get().status?.locked) await get().refresh();
    // Two atomics and two `stat` calls, so the poll is what keeps savePending,
    // saveBlocked and locksInMs honest while the vault sits idle.
    setInterval(() => {
      void get().refreshStatus();
    }, STATUS_POLL_MS);
  },

  refreshStatus: async () => {
    try {
      set({ status: await vault.vaultStatus() });
    } catch {
      // `vault_status` answers `{ locked: true }` for a locked vault rather
      // than rejecting, so a rejection here is a transport failure. Keep the
      // last known save state: zeroing it would hide a banner the user can
      // still see and act on.
      const previous = get().status;
      set({
        status: {
          exists: previous?.exists ?? false,
          locked: true,
          savePending: previous?.savePending ?? false,
          saveBlocked: previous?.saveBlocked ?? false,
          backupAt: previous?.backupAt ?? null,
          locksInMs: null,
        },
      });
    }
  },

  refresh: async () => {
    let list: { entries: EntrySummary[]; groups: Group[] };
    try {
      list = await vault.vaultList();
    } catch {
      // Locked or otherwise unavailable: the locked event owns clearing.
      return;
    }
    const { query } = get();
    let searchIds: string[] | null = null;
    if (query.trim().length > 0) {
      try {
        searchIds = await vault.vaultSearch(query);
      } catch {
        searchIds = null;
      }
      // The user may have typed while this search was in flight; the newer
      // `setQuery` result is the one that describes what the list shows.
      if (get().query !== query) searchIds = get().searchIds;
    }
    set({ entries: list.entries, groups: list.groups, searchIds });
    // A selection the current scope, tag filter or query no longer shows must
    // not keep a stale row in the detail pane.
    get().pruneSelection();
    await get().refreshDetail();
  },

  refreshDetail: async () => {
    const id = get().selectedId;
    if (id === null) {
      set({ detail: null });
      return;
    }
    try {
      const detail = await vault.vaultEntryGet(id);
      if (get().selectedId === id) set({ detail });
    } catch {
      if (get().selectedId === id) set({ detail: null });
    }
  },

  unlock: async (masterPassword) => {
    await vault.vaultUnlock(masterPassword);
    await get().refreshStatus();
    await get().refresh();
  },

  create: async (masterPassword) => {
    await vault.vaultCreate(masterPassword);
    await get().refreshStatus();
    await get().refresh();
  },

  lock: async () => {
    await vault.vaultLock();
  },

  selectScope: (scope) => {
    if (get().scope === scope) return;
    set({ scope, selectedId: null, detail: null });
  },

  selectEntry: (id) => {
    set({ selectedId: id });
    void get().refreshDetail();
  },

  pruneSelection: () => {
    const state = get();
    if (state.selectedId === null) return;
    const visible = visibleEntries({
      entries: state.entries,
      scope: state.scope,
      tagFilter: state.tagFilter,
      searchIds: state.searchIds === null ? null : new Set(state.searchIds),
      now: Date.now(),
    });
    if (visible.some((entry) => entry.id === state.selectedId)) return;
    set({ selectedId: null });
    void get().refreshDetail();
  },

  setQuery: async (query) => {
    set({ query });
    if (query.trim().length === 0) {
      set({ searchIds: null });
      get().pruneSelection();
      return;
    }
    let ids: string[];
    try {
      ids = await vault.vaultSearch(query);
    } catch {
      return;
    }
    // Drop a response for a query the user has already moved past.
    if (get().query !== query) return;
    set({ searchIds: ids });
    get().pruneSelection();
  },

  toggleTag: (key) => {
    const lower = key.toLowerCase();
    const current = get().tagFilter;
    set({
      tagFilter: current.includes(lower)
        ? current.filter((tag) => tag !== lower)
        : [...current, lower],
    });
    get().pruneSelection();
  },

  clearTags: () => {
    set({ tagFilter: [] });
    get().pruneSelection();
  },

  toggleExpanded: (id) => {
    const expanded = get().expanded;
    set({
      expanded: expanded.includes(id) ? expanded.filter((item) => item !== id) : [...expanded, id],
    });
  },

  openEditor: (entryId, groupId) => set({ editor: { entryId, groupId } }),

  closeEditor: () => set({ editor: null }),

  dismissQuit: () => set({ quitPrompt: false }),

  retrySave: async () => {
    await vault.vaultRetrySave();
    await get().refreshStatus();
  },

  restoreSnapshot: async () => {
    await vault.vaultRestoreSnapshot();
    await get().refreshStatus();
    await get().refresh();
  },

  touchActivity: () => {
    const now = Date.now();
    if (now - lastTouchAt < TOUCH_INTERVAL_MS) return;
    lastTouchAt = now;
    void vault.vaultTouch().catch(() => {});
  },
}));
