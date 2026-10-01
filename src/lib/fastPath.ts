/**
 * localStorage fast-path shadow helpers. Several first-paint paths mirror a
 * persisted preference into localStorage and read it back before React mounts;
 * these are the shared get/set with the availability guards.
 */

/** Read a shadow. Null when the key is unset or localStorage is unavailable. */
export function readShadow(key: string): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** Write a shadow, or remove it when `value` is null. */
export function writeShadow(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // ignore: localStorage may be unavailable in some embeddings.
  }
}
