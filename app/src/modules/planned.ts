import { icons } from "@/components/Icon";
import type { TetherModule } from "./types";

/**
 * Features that are designed but not built, in the order agreed in
 * docs/ROADMAP.md ("Phases"). They appear on the Modules page so the app shows
 * where it is going; each becomes a real module by adding a `view` and moving
 * it into `registry.ts`. (Drop, the shared shelf, will grow out of the
 * Transfers module, and Handoff out of Workspaces.)
 */
export const plannedModules: TetherModule[] = [
  {
    id: "notes",
    label: "Notes",
    icon: icons.edit,
    group: "Workspace",
    summary: "Notes that merge live across your computers, offline included, with history and who wrote what.",
    status: "planned",
    note: "Next · F11",
  },
  {
    id: "pulse",
    label: "Pulse",
    icon: icons.pulse,
    group: "Network",
    summary: "Every computer's health at a glance: online, CPU, memory, storage, battery, jobs. Wake a sleeping one.",
    status: "planned",
    note: "Roadmap · P3",
  },
  {
    id: "compute",
    label: "Compute",
    icon: icons.flash,
    group: "Network",
    summary: "Run a supported job on your strongest computer from any other, watch it, cancel it.",
    status: "planned",
    note: "Roadmap · P3",
  },
  {
    id: "drive",
    label: "Drive",
    icon: icons.folderSync,
    group: "Sync",
    summary: "Open folders shared from another computer; files download only when opened.",
    status: "planned",
    note: "Roadmap · P4",
  },
  {
    id: "rules",
    label: "Rules",
    icon: icons.wand,
    group: "Workspace",
    summary: "When something happens on one computer, do something on another. Each event runs once.",
    status: "planned",
    note: "Roadmap · P6",
  },
  {
    id: "timeline",
    label: "Timeline",
    icon: icons.clock,
    group: "Workspace",
    summary: "Scroll back through what happened on your computers and restore it.",
    status: "planned",
    note: "Roadmap · P6",
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
    id: "spaces",
    label: "Spaces",
    icon: icons.people,
    group: "Workspace",
    summary: "Share a space with friends or teammates, with its own keys and offline edits.",
    status: "planned",
    note: "Roadmap · P8",
  },
];
