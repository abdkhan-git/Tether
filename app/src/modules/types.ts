import type { ComponentType } from "react";
import type { IconData } from "@/components/Icon";

export type ModuleGroup = "Sync" | "Workspace" | "Network";

/** Something a module adds to the search flyout. */
export interface Command {
  id: string;
  label: string;
  keywords?: string;
  run: () => void;
}

/**
 * A feature of Tether. Adding a feature means writing one of these and adding it
 * to `registry.ts`. The top navigation, the search flyout and the Modules page
 * are all generated from the registry, so nothing else needs to change.
 */
export interface TetherModule {
  id: string;
  label: string;
  icon: IconData;
  group: ModuleGroup;
  summary: string;
  /** `planned` modules have no view yet and show up on the Modules page only. */
  status: "stable" | "preview" | "planned";
  /** Roadmap stage or origin note, shown on the Modules page. */
  note?: string;
  /** Core modules cannot be switched off. */
  core?: boolean;
  /**
   * The daemon implements what this module shows. Modules without it appear
   * only in the browser preview; the desktop app lists them as coming.
   */
  daemon?: boolean;
  view?: ComponentType;
  commands?: () => Command[];
}
