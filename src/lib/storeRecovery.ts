import { reason, tauriStoreFileIo } from "./storeFileIo";
import {
  inspect,
  SNAPSHOT_SUFFIX,
  storeFilePaths,
  type StoreFileIo,
  type StoreFileState,
} from "./storeFileState";

// Crash recovery for a JSON store file: the policy, over a port.
//
// The failure this exists for: a store file left zero-length or nul-filled by a
// power cut (plugins-workspace#3085) comes back as an EMPTY store, because a
// store layer that cannot parse the file has nothing else to report, and the
// next save writes that emptiness over whatever was left of it.
//
// Survivable for a saved layout or a theme. Not survivable for credentials: the
// secret store writes atomically, so a lost store file leaves the private key in
// the keychain with no record naming it - bytes no code path can enumerate or
// delete.
//
// The mitigation is a check before the first read plus a `.bak` snapshot after
// each good SAVE - not after each good load, which would leave the session that
// CREATES the file with no snapshot at all, since at first load there is nothing
// to copy. It needs no new Rust: `fs_read_file` and `fs_write_file` already exist,
// and the latter goes through the app's own atomic temp-plus-rename path.
//
// WHICH FILES. The app's store files go through `createRecoveredStore` and
// therefore through here; settings is the only one left. It gets a corruption
// check before its first read, a `.bak` snapshot after every commit, and a
// whole-file atomic write.
//
// EVERY function here is total: it reports a filesystem it could not work with
// instead of rejecting. A caller that caches the promise of this work - which is
// the only sane way to run it once - would otherwise cache a REJECTION, and then
// a single transient failure disables the store for the rest of the process. The
// module whose whole purpose is coping with a filesystem in a bad state must not
// fail worse than not having it.
//
// What a file's bytes MEAN is `./storeFileState`'s question, and the Tauri side
// of the port is `./storeFileIo`'s, so this file stays policy and imports no
// Tauri. The one platform detail worth spelling out lives with that adapter: a
// store path resolves against `appDataDir()`, secrets against
// `app_local_data_dir()`, and those are DIFFERENT directories on Windows, so
// nothing here may be reused to reach a secret file.

export type StoreRecovery = {
  /**
   * What the primary looked like before anything was done to it, with ONE
   * deliberate exception: an absent primary beside a snapshot that could not be
   * read reports `"unreadable"` rather than `"missing"`, because the pair's
   * verdict is what a caller acts on and "there is nothing here" was not
   * established. See the branch in `recover` for what acting on the wrong one
   * costs.
   */
  found: StoreFileState;
  /** True when the snapshot was copied over the primary. */
  recovered: boolean;
  /**
   * Worth telling the user about, or absent when nothing is. A caller shows this
   * as a toast; `src/lib` deliberately does not import one, so the notice travels
   * instead of the dependency.
   */
  note?: string;
};

/** What one snapshot attempt did. */
export type StoreSnapshot = {
  /** True when the snapshot was written. False when the primary was not good
   *  enough to copy - which is the guard working, not a failure. */
  taken: boolean;
  /** Set only when the snapshot could not be WRITTEN, so a caller can say that
   *  the safety net is missing rather than assume it is there. */
  note?: string;
};

/**
 * Put a usable store file in place, or report why there isn't one.
 *
 * MUST run before the store is first touched. Every store layer over this one
 * caches what its first read found, so by the time a read comes back empty the
 * store has already decided the file was worthless and the next save writes
 * that emptiness over it.
 *
 * Never rejects - see the module header.
 */
export async function recoverStoreFile(
  fileName: string,
  io: StoreFileIo = tauriStoreFileIo,
): Promise<StoreRecovery> {
  try {
    return await recover(fileName, io);
  } catch (e) {
    return {
      found: "unreachable",
      recovered: false,
      note: `${fileName} could not be checked: ${reason(e)}`,
    };
  }
}

async function recover(fileName: string, io: StoreFileIo): Promise<StoreRecovery> {
  const { primary, snapshot } = storeFilePaths(await io.dir(), fileName);
  const primaryRead = await io.read(primary);
  const found = inspect(primaryRead);
  if (found === "ok") return { found, recovered: false };
  if (primaryRead.kind === "unreadable") {
    // The file is THERE and would not open. Its bytes are unknown, so they may
    // be perfectly good - which makes restoring a snapshot over them the same
    // destruction as restoring over a too-large file, for the same reason: a
    // read this app could not make is not evidence about the contents.
    return {
      found,
      recovered: false,
      note: `${fileName} could not be read, so it was left as it is: ${primaryRead.reason}`,
    };
  }
  if (found === "toolarge") {
    // Not corruption: the plugin has no size limit and reads this fine. Say so
    // and stop - restoring a snapshot over it would destroy real data.
    return {
      found,
      recovered: false,
      note: `${fileName} is too large to check for corruption; left as it is`,
    };
  }

  const backup = await io.read(snapshot);
  const fallback = inspect(backup);
  if (backup.kind !== "text" || fallback !== "ok") {
    // Both absent is a first run, not a loss: say nothing.
    if (found === "missing" && fallback === "missing") return { found, recovered: false };
    // An absent primary beside a snapshot that could not be READ is NOT a first
    // run, and reporting it as one is a data-loss path rather than a wording
    // choice: a caller that seeds a default on "missing" persists it, and the
    // snapshot taken after that commit copies the default over the very `.bak`
    // whose contents nothing here ever saw. So the pair's verdict is the
    // snapshot's, which is the only honest thing to say - nothing was
    // established about what is here.
    if (found === "missing" && (fallback === "unreadable" || fallback === "toolarge")) {
      return {
        found: "unreadable",
        recovered: false,
        note: `${fileName} is missing and its snapshot is ${fallback}, so nothing here could be checked`,
      };
    }
    return {
      found,
      recovered: false,
      note: `${fileName} is ${found} and its snapshot is ${fallback}`,
    };
  }

  // The snapshot is metadata from the last process start; the SECRET store is
  // atomic and therefore current. So a restore can leave the two disagreeing: a
  // key whose fingerprint was rotated after the snapshot comes back naming the
  // OLD fingerprint while the keychain holds the new PEM, and the import dedupe
  // is on fingerprint - so importing the original key would "find" that record
  // and bind a host to the wrong material. Presence flags drift the same way,
  // permanently, because they are deliberately never read back. Nothing here
  // reconciles the two, and nothing above it does either.
  try {
    await io.write(primary, backup.content);
  } catch (e) {
    return {
      found,
      recovered: false,
      note: `${fileName} is ${found} and could not be restored from ${fileName}${SNAPSHOT_SUFFIX}: ${reason(e)}`,
    };
  }
  return {
    found,
    recovered: true,
    note: `${fileName} was ${found}; restored from ${fileName}${SNAPSHOT_SUFFIX}`,
  };
}

/**
 * Snapshot a store file that is currently good, over a snapshot worth replacing.
 *
 * TWO guards, and they refuse in opposite directions. The primary must be good,
 * or a torn file would be copied over the last good copy and a recoverable crash
 * would become a total loss. And the EXISTING snapshot must be one this process
 * could read: bytes nobody managed to look at may be perfectly good ones, and
 * they are the only copy left whenever the primary is absent or torn. That is
 * the same rule `recover` applies to a primary it could not read, applied to the
 * other file.
 *
 * The second guard costs one extra read PER COMMIT. `snapshotAfterSave` looks
 * like it would fold a burst into one pass, and it does not: it coalesces only
 * callers that overlap, and every store layer commits inside `enqueueWrite`, so
 * they never do. Measured rather than reasoned about, because the first version
 * of this comment claimed the cheaper number.
 *
 * Worth it anyway, and the trade is not close: one read against replacing the
 * only surviving copy of a store file. It is also the ONLY thing that can notice
 * a snapshot going unreadable mid-session - `recover` runs once, at startup, and
 * does not even look at the snapshot when the primary is good.
 *
 * A caller may still fire this after any save without checking anything first.
 *
 * Never rejects - see the module header.
 */
export async function snapshotStoreFile(
  fileName: string,
  io: StoreFileIo = tauriStoreFileIo,
): Promise<StoreSnapshot> {
  try {
    const { primary, snapshot } = storeFilePaths(await io.dir(), fileName);
    const read = await io.read(primary);
    if (read.kind !== "text" || inspect(read) !== "ok") return { taken: false };

    // `missing` is the ordinary case - a first snapshot has nothing to protect -
    // and every other unreadable answer is a file whose contents are unknown.
    const existing = await io.read(snapshot);
    if (existing.kind === "unreadable" || existing.kind === "toolarge") {
      return {
        taken: false,
        note: `${fileName}${SNAPSHOT_SUFFIX} could not be read, so it was left as it is rather than replaced`,
      };
    }

    await io.write(snapshot, read.content);
    return { taken: true };
  } catch (e) {
    return {
      taken: false,
      note: `${fileName}${SNAPSHOT_SUFFIX} could not be written: ${reason(e)}`,
    };
  }
}
