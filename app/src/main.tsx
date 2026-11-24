import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { inDesktopApp } from "./data/daemon";
import { startMockDaemon } from "./data/store";

// Inside the desktop app the data comes from tetherd. In a plain browser
// (`npm run dev`) there is no daemon, so preview data stands in for it.
// The quick paste palette is its own window (label "palette"), or `?palette`
// in the browser preview.
async function start() {
  let palette = new URLSearchParams(location.search).has("palette");
  if (inDesktopApp) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    palette = getCurrentWindow().label === "palette";
  }
  if (palette) {
    const { mountPalette } = await import("./palette/PaletteApp");
    mountPalette();
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
