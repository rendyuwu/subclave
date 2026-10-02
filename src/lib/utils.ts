import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Lowercase, hyphenate runs of non-alphanumerics, trim leading/trailing
 *  hyphens. Returns `fallback` (default "") when the result is empty. */
export function slugify(name: string, fallback = ""): string {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || fallback
  );
}
