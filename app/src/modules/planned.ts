import { icons } from "@/components/Icon";
import type { TetherModule } from "./types";

/**
 * Features that are designed but not built. They appear on the Modules page so
 * the app shows where it is going; each becomes a real module by adding a
 * `view` and moving it into `registry.ts`.
 *
 * "Proposed" ones are not in docs/DESIGN.md yet and need a design pass first.
 */
export const plannedModules: TetherModule[] = [
  {
    id: "spaces",
    label: "Spaces",
    icon: icons.people,
    group: "Workspace",
    summary: "Share a space with friends or teammates, with its own keys and offline edits.",
    status: "planned",
    note: "Roadmap · Stage 5",
  },
  {
    id: "vault",
    label: "Vault",
    icon: icons.key,
    group: "Sync",
    summary: "Move .env files and API keys between your devices. Never enters clipboard history, and expires on its own.",
    status: "planned",
    note: "Proposed",
  },
  {
    id: "drop-folders",
    label: "Drop folders",
    icon: icons.folderSync,
    group: "Sync",
    summary: "Point a folder at a device: new screenshots or exports land there on their own.",
    status: "planned",
    note: "Proposed",
  },
  {
    id: "remote-actions",
    label: "Remote actions",
    icon: icons.flash,
    group: "Network",
    summary: "Lock, sleep or open a link on another device. Signed operations, allow-listed actions only.",
    status: "planned",
    note: "Proposed",
  },
];
