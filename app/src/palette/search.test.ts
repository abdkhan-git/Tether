import { describe, expect, it } from "vitest";
import type { Clip } from "@/data/types";
import { matches } from "./search";

const clip = (text: string, sensitive = false): Clip => ({ id: "a:1", device: "a", ts: 0, text, pinned: false, sensitive });

describe("palette search", () => {
  it("matches every word, in the text or the device name, in any case", () => {
    expect(matches(clip("SELECT * FROM users"), "Desktop", "select users")).toBe(true);
    expect(matches(clip("SELECT * FROM users"), "Desktop", "users desktop")).toBe(true);
    expect(matches(clip("SELECT * FROM users"), "Desktop", "select orders")).toBe(false);
    expect(matches(clip("anything"), "Laptop", "   ")).toBe(true);
  });

  it("never matches on the text of a sensitive item", () => {
    expect(matches(clip("sk-live-abc123", true), "Laptop", "abc123")).toBe(false);
    expect(matches(clip("sk-live-abc123", true), "Laptop", "laptop")).toBe(true);
  });
});
