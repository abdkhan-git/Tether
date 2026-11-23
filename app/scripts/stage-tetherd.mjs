// Builds tetherd in release mode and puts it where Tauri's bundler expects a
// sidecar: src-tauri/binaries/tetherd-<target triple>[.exe]. The installer
// then places it next to the app's executable, where the app looks for it.
// Run by `npm run app:build`; the staged file is git-ignored.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const app = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = join(app, "..");

const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .split("\n")
  .find((line) => line.startsWith("host: "))
  ?.slice("host: ".length)
  .trim();
if (!host) throw new Error("could not read the host target from `rustc -vV`");

execFileSync("cargo", ["build", "--release", "-p", "tether-daemon"], { cwd: root, stdio: "inherit" });

const exe = process.platform === "win32" ? ".exe" : "";
const target = process.env.CARGO_TARGET_DIR ?? join(root, "target");
const to = join(app, "src-tauri", "binaries", `tetherd-${host}${exe}`);
mkdirSync(dirname(to), { recursive: true });
copyFileSync(join(target, "release", `tetherd${exe}`), to);
console.log(`staged ${to}`);
