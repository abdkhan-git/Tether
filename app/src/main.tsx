import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { inDesktopApp } from "./data/daemon";
import { startMockDaemon } from "./data/store";

// Inside the desktop app the data comes from tetherd. In a plain browser
// (`npm run dev`) there is no daemon, so preview data stands in for it.
// The quick paste palette and the scratchpad are windows of their own (labels
// "palette" and "scratchpad"), or `?palette` / `?scratchpad` in the browser preview.
async function start() {
  const params = new URLSearchParams(location.search);
  let label = params.has("palette") ? "palette" : params.has("scratchpad") ? "scratchpad" : "main";
  if (inDesktopApp) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    label = getCurrentWindow().label;
  }
  if (label === "palette") {
    const { mountPalette } = await import("./palette/PaletteApp");
    mountPalette();
    return;
  }
  if (label === "scratchpad") {
    const { mountScratchpad } = await import("./scratchpad/ScratchpadApp");
    mountScratchpad();
    return;
  }
  if (inDesktopApp) {
    void import("./data/live").then((live) => live.connectDaemon());
  } else {
    startMockDaemon();
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void start();
