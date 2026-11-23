import { clipboardModule } from "./clipboard";
import { devicesModule } from "./devices";
import { diagnosticsModule } from "./diagnostics";
import { overviewModule } from "./overview";
import { transfersModule } from "./transfers";
import { workspacesModule } from "./workspaces";
import type { TetherModule } from "./types";

/**
 * Every module that has a view, in navigation order. To add a feature: write a
 * `TetherModule` and add it here. The top navigation, the search flyout and the
 * Modules page pick it up on their own. (The Modules page itself lives outside
 * this list; the grid button in the title bar opens it.)
 */
export const liveModules: TetherModule[] = [overviewModule, clipboardModule, transfersModule, workspacesModule, devicesModule, diagnosticsModule];
