import type { Clip } from "@/data/types";

/**
 * Does `clip` match the palette query? Every word must appear in the text or
 * in the name of the device it came from. Sensitive items match only by
 * device name: their text is never searched, as it is never shown.
 */
export function matches(clip: Clip, from: string, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${clip.sensitive ? "" : clip.text.toLowerCase()} ${from.toLowerCase()}`;
  return words.every((w) => hay.includes(w));
}
