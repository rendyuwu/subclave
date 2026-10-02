import { invoke } from "@tauri-apps/api/core";
import { appDataDir } from "@tauri-apps/api/path";

import type { FsReadResult } from "./ipc";
import { classifyReadFailure, type StoreFileIo } from "./storeFileState";

// The Tauri implementation of the store file port, plus the error wording the
// rest of the recovery code reports with. Kept apart from `./storeRecovery` so
// that file is policy over a port rather than policy tangled up with `invoke`,
// and so the classification it leans on stays loadable under plain node.
//
// One platform detail worth spelling out: a store path resolves against
// `appDataDir()`. Secrets resolve against `app_local_data_dir()`. Those are the
// same directory on Linux and DIFFERENT ones on Windows, so nothing here may be
// reused to reach a secret file.
//
// Neither function rejects on the caller's behalf: a read that failed is sorted
// into `missing` or `unreadable`, which is a state the recovery pass decides
// about, not an error it propagates.

/** The message out of whatever a `catch` caught. */
export function reason(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export const tauriStoreFileIo: StoreFileIo = {
  dir: () => appDataDir(),
  read: async (path) => {
    try {
      const result = await invoke<FsReadResult>("fs_read_file", { path });
      switch (result.kind) {
        case "text":
          return { kind: "text", content: result.content };
        case "toolarge":
          return { kind: "toolarge" };
        default:
          return { kind: "binary" };
      }
    } catch (e) {
      return classifyReadFailure(reason(e));
    }
  },
  write: (path, content) => invoke<void>("fs_write_file", { path, content }),
};
