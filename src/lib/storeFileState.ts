// What a store file's bytes mean, and where its snapshot lives: the pure half of
// crash recovery, split out from `./storeRecovery` so the classification can be
// exercised under plain node - nothing here imports Tauri - and so the recovery
// policy next door reads as policy rather than as byte sniffing.
//
// The failure this exists for: a store file left zero-length or nul-filled by a
// power cut (plugins-workspace#3085) comes back as an EMPTY store, because a
// store layer that cannot parse the file has nothing else to report, and the
// next save writes that emptiness over whatever was left of it.
//
// The port below is the seam: the Tauri implementation of it, and the one
// platform detail about which directory a store path resolves against, live in
// `./storeFileIo`.

/** Snapshot taken beside the store file it protects. */
export const SNAPSHOT_SUFFIX = ".bak";

/**
 * How a store file looks on disk.
 *
 * `"ok"` is the only trustworthy answer. `"missing"`, `"empty"`, `"nul"` and
 * `"unparseable"` all mean the snapshot should be preferred. The last THREE mean
 * nothing can be decided, so nothing may be written over the primary either:
 * `"toolarge"` is a file `fs_read_file` will not return for its size,
 * `"unreadable"` is one it would not open at all, and `"unreachable"` is the
 * data directory itself failing.
 */
export type StoreFileState =
  "ok" | "missing" | "empty" | "nul" | "unparseable" | "toolarge" | "unreadable" | "unreachable";

/**
 * One file as the host process can see it.
 *
 * `binary` is what `fs_read_file` reports for a nul-filled file: its null-byte
 * sniff classifies the buffer as binary rather than returning the bytes, and
 * that classification IS the corruption being looked for. An `image`
 * classification lands here too - unreachable for a `.json` path in practice,
 * but it is the same "bytes where JSON should be" class.
 *
 * `toolarge` is deliberately NOT folded in with it. A 10 MB store file is real
 * data the plugin reads fine; calling it corruption would restore a snapshot
 * over it.
 *
 * `unreadable` is the same distinction one step further out, and it exists
 * because folding it into `missing` is a data-loss bug rather than a rounding
 * error: a file that is THERE and would not open (a lock during an update
 * handoff, a sharing violation, EACCES, a descriptor limit) must not be treated
 * as a first run. Everything downstream writes over a first run - the recovery
 * pass would restore a snapshot, and a store layer would come up empty and save
 * that emptiness over a seeded default. `missing` therefore means "the OS said
 * there is no such file", never "the read did not work out".
 */
export type StoreFileRead =
  | { kind: "text"; content: string }
  | { kind: "binary" }
  | { kind: "toolarge" }
  | { kind: "unreadable"; reason: string }
  | { kind: "missing" };

/**
 * The filesystem operations recovery needs.
 *
 * An injectable port because `scripts/*-verify.ts` runs under plain node with no
 * Tauri runtime, and a torn store file - or a write that fails - is the one thing
 * here that cannot be reproduced by hand on a real machine. The Tauri
 * implementation lives in `./storeFileIo`.
 */
export type StoreFileIo = {
  /** Directory a store path resolves against, which is `appDataDir()`. */
  dir(): Promise<string>;
  read(path: string): Promise<StoreFileRead>;
  /**
   * Write `content` over `path`, creating or replacing it.
   *
   * A write rather than a copy, on purpose: both callers here have already READ
   * and validated the bytes they want in place, so `fs_write_file` does the
   * whole job in one command, through the app's atomic temp-plus-rename path.
   * A copy-based port would instead need the old file removed first, and a
   * delete that fails (an antivirus or indexer holding the handle on Windows, a
   * read-only data directory) would turn "the good snapshot is sitting right
   * there" into an error.
   */
  write(path: string, content: string): Promise<void>;
};

/** The primary path and the snapshot path resolved beside it. */
export type StoreFilePaths = { primary: string; snapshot: string };

/**
 * Where a store file and its snapshot live.
 *
 * Exported because `lib/fileKeyValueStore.ts` resolves the SAME primary path to
 * read and write it. Two derivations of `${dir}/${fileName}` would be two
 * conventions, and the day they disagreed the recovery pass would be checking a
 * different file from the one the store reads.
 */
export function storeFilePaths(dir: string, fileName: string): StoreFilePaths {
  // `appDataDir()` carries no trailing separator today; normalise anyway so a
  // future change cannot produce a double slash. Forward slashes are fine on
  // Windows - Rust's `PathBuf` accepts either.
  const primary = `${dir.replace(/[\\/]+$/, "")}/${fileName}`;
  return { primary, snapshot: primary + SNAPSHOT_SUFFIX };
}

/**
 * The verdict for one file: what the read actually says, with an unusable
 * payload of any kind (empty, all-nul, unparseable, not a key/value map)
 * reported as its own state rather than as the store layer's problem.
 *
 * Total, like everything here: a file it cannot make sense of is a state, not a
 * rejection.
 */
export function inspect(read: StoreFileRead): StoreFileState {
  if (read.kind === "missing") return "missing";
  if (read.kind === "binary") return "nul";
  if (read.kind === "toolarge") return "toolarge";
  if (read.kind === "unreadable") return "unreadable";
  // `trim` does not strip U+0000, so an all-nul buffer falls through to the
  // check below rather than reading as merely empty.
  if (read.content.trim() === "") return "empty";
  if (/^\0+$/.test(read.content)) return "nul";
  try {
    const parsed: unknown = JSON.parse(read.content);
    // A store file's top level is the key/value map, so an array or a bare
    // scalar is as unusable as a syntax error - and far likelier to be the tail
    // of a torn write that happened to parse.
    const usable = parsed !== null && typeof parsed === "object" && !Array.isArray(parsed);
    return usable ? "ok" : "unparseable";
  } catch {
    return "unparseable";
  }
}

/**
 * Turn a rejected `fs_read_file` into "there is no such file" or "there is a
 * file and it would not open".
 *
 * The command rejects for BOTH, so the two are told apart here or not at all -
 * and telling them apart is the whole of what stops a file that merely would not
 * open being written over as though it were a first run.
 *
 * The message is the entire contract: `fs_read_file` returns
 * `Result<_, String>`, so nothing structured survives the boundary. Rust's
 * `io::Error` Display always appends `(os error N)` for an OS error, on every
 * platform, whatever the localised text before it says - 2 is ENOENT and
 * Windows' ERROR_FILE_NOT_FOUND, 3 is Windows' ERROR_PATH_NOT_FOUND, and a
 * missing parent directory on a first run gives one of those. Anything else -
 * EACCES, a sharing violation, a descriptor limit, the command's own join error
 * - is a file this process did not get to see.
 * `missing_file_error_carries_the_os_error_suffix` in
 * `src-tauri/src/modules/fs/file.rs` is what holds the other end of it.
 *
 * ANCHORED to the end, which `to_string()` always is. Unanchored would match the
 * suffix appearing anywhere in a wrapped or chained message, and that is the
 * direction that turns an unreadable file back into a first run.
 *
 * Exported so the rule itself is checkable: the Tauri port in `./storeFileIo`
 * cannot run under node, and a verify script re-implementing this regex would be
 * a copy free to drift from the one that ships.
 */
export function classifyReadFailure(message: string): StoreFileRead {
  // The unrecognised case falls to `unreadable`, which is the safe direction:
  // the cost of calling a first run unreadable is a default that waits for the
  // first real change, and the cost of the reverse is the file.
  return /\(os error (?:2|3)\)$/.test(message.trim())
    ? { kind: "missing" }
    : { kind: "unreadable", reason: message };
}
