/**
 * The toast text for a copy action. `clearsAt` is the epoch ms the clipboard
 * clears (`clip_copy_field`'s answer); `null` means it never will, so the
 * countdown is omitted. `now` is a parameter so the caller and any check agree
 * on the same instant.
 */
const LABELS: Record<string, string> = {
  password: "Password",
  username: "Username",
  totp: "Code",
};

export function copyToastText(
  kind: "password" | "username" | "totp" | string,
  clearsAt: number | null,
  now: number,
): string {
  const message = `${LABELS[kind] ?? kind} copied.`;
  if (clearsAt === null) return message;
  const seconds = Math.max(0, Math.ceil((clearsAt - now) / 1000));
  return `${message} Clears in ${seconds} second${seconds === 1 ? "" : "s"}.`;
}
