// Random but repeatable inputs for the notes tests.

import { fromSteps, type Delta, type Step } from "./delta";

/** A small repeatable random source. */
export function rng(seed: number) {
  let s = seed >>> 0;
  return (n: number) => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return Math.floor((s / 2 ** 32) * n);
  };
}

const letters = ["a", "b", "c", "é", "😀", "x", " ", "漢", "\n"];
export function word(r: (n: number) => number, max: number) {
  return Array.from({ length: r(max + 1) }, () => letters[r(letters.length)]).join("");
}

/** A random delta that fits `text` and never splits a character. */
export function randomDelta(r: (n: number) => number, text: string): Delta {
  const chars = [...text];
  const steps: Step[] = [];
  for (let i = 0; i < 5; i++) {
    const n = Math.min(r(4), chars.length);
    const units = chars.slice(0, n).join("").length;
    const kind = r(3);
    if (kind === 0) steps.push(word(r, 3));
    else {
      steps.push(kind === 1 ? units : -units);
      chars.splice(0, n);
    }
  }
  return fromSteps(steps);
}
