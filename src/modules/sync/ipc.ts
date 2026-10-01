// The thin IPC wrappers for the sync commands, and the bundle the scheduler
// takes as a port.
//
// THE ONLY FILE IN THIS MODULE THAT IMPORTS `invoke`. Every command name is a
// literal at its call, on the same line, because the command-registry scanner
// reads it there: a name behind a variable reads as a dynamic site and has to be
// enumerated by hand instead.

import { invoke } from "@tauri-apps/api/core";

import type {
  SyncCommands,
  SyncConfigureArgs,
  SyncConfigureResult,
  SyncJoinArgs,
  SyncJoinResult,
  SyncPullResult,
  SyncPushResult,
} from "./types";

export function syncConfigure(args: SyncConfigureArgs): Promise<SyncConfigureResult> {
  return invoke<SyncConfigureResult>("sync_configure", { args });
}

export function syncDisable(): Promise<void> {
  return invoke<void>("sync_disable");
}

export function syncPull(): Promise<SyncPullResult> {
  return invoke<SyncPullResult>("sync_pull");
}

export function syncPush(): Promise<SyncPushResult> {
  return invoke<SyncPushResult>("sync_push");
}

export function syncJoin(args: SyncJoinArgs): Promise<SyncJoinResult> {
  return invoke<SyncJoinResult>("sync_join", { args });
}

/** The five commands, bundled for the scheduler's injected port. */
export const syncCommands: SyncCommands = {
  configure: syncConfigure,
  disable: syncDisable,
  pull: syncPull,
  push: syncPush,
  join: syncJoin,
};
