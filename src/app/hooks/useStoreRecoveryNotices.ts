import { useEffect, useRef } from "react";

import { toast } from "@/components/ui/toast";
import {
  ensureLoaded as ensureSettingsLoaded,
  onSettingsStoreChanged,
  takeRecoveryNotice as takeSettingsRecoveryNotice,
} from "@/modules/settings/store";
import {
  announceRecovery,
  drainRecovery,
  type RecoverableStore,
  type Say,
} from "../lib/recoveryNotices";

/**
 * The stores whose recovery notice is SAID.
 *
 * Hand-maintained, and that is the whole hazard: `createRecoveredStore` produces
 * a notice for every store built on it, so a new one is silent until somebody
 * remembers this list. `scripts/recovery-notice-verify.ts` asserts set EQUALITY
 * between the modules under `src/modules` that export `takeRecoveryNotice` and
 * the modules named here, so the next one cannot be forgotten.
 */
const STORES: RecoverableStore[] = [
  {
    // What the window that edits them is called.
    label: "Settings",
    ensureLoaded: ensureSettingsLoaded,
    takeRecoveryNotice: takeSettingsRecoveryNotice,
    onChanged: onSettingsStoreChanged,
  },
];

const say: Say = (t) => toast(t.message, { variant: t.variant });

/**
 * Tell the user when a store came back from its `.bak` - the half of crash
 * recovery that was missing. `createRecoveredStore` has always produced the
 * notice; until this hook, nothing in `src/` ever asked for it, so a recovery
 * was completely silent.
 *
 * Fired and forgotten: it gates nothing and cannot reject (see
 * `announceRecovery`). The policy lives in `app/lib/recoveryNotices.ts`; this is
 * the mount and the real stores.
 *
 * A second toast cannot happen even across a genuine unmount/remount of App -
 * but `startedRef` is NOT why. It is a `useRef`, so a real remount gets a
 * fresh ref and would ask again; it only stops THIS mount's effect from
 * asking twice within its own lifetime (e.g. a React StrictMode dev
 * double-invoke of the same effect). What actually makes the guarantee hold
 * across a remount is store-side: `ensureLoaded()` DRAINS the notice slot
 * (`createRecoveredStore`'s `takeRecoveryNotice`, in `lib/recoveredStore.ts`)
 * before returning it, so every ask after the first - whichever ref asked -
 * finds the slot already empty and says nothing. The change listeners are
 * re-established on a remount instead, because those have to follow the
 * mount rather than the launch.
 */
export function useStoreRecoveryNotices(): void {
  const startedRef = useRef(false);
  useEffect(() => {
    if (!startedRef.current) {
      startedRef.current = true;
      for (const store of STORES) void announceRecovery(store, say);
    }

    const unlisteners: (() => void)[] = [];
    let disposed = false;
    for (const store of STORES) {
      void store
        .onChanged(() => drainRecovery(store, say))
        .then((off) => {
          if (disposed) off();
          else unlisteners.push(off);
        })
        .catch((e: unknown) => {
          console.error(`${store.label}: could not listen for changes`, e);
        });
    }

    return () => {
      disposed = true;
      for (const off of unlisteners) off();
    };
  }, []);
}
