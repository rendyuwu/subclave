// Rust errors are `"<module>: <sentence>"`, lowercase. The webview shows them
// as sentences: strip the module prefix, capitalise the first letter and end
// with a full stop, so the unlock screen, the editor and the group dialogs all
// read the same way. Two messages the unlock flow fixes verbatim are mapped
// below.

const PREFIX_RE = /^[a-z][a-z0-9_-]*:\s*/;

const VERBATIM: Record<string, string> = {
  "this vault was written by a newer Subclave":
    "This vault was written by a newer Subclave. Update to open it.",
  "wrong master password, or the vault file is corrupt":
    "Wrong master password, or the vault file is corrupt.",
};

export function describeVaultError(message: string): string {
  const body = message.trim().replace(PREFIX_RE, "");
  const verbatim = VERBATIM[body.replace(/\.+$/, "")];
  if (verbatim) return verbatim;
  if (body.length === 0) return message;
  const sentence = body.charAt(0).toUpperCase() + body.slice(1);
  return /[.!?]$/.test(sentence) ? sentence : `${sentence}.`;
}
