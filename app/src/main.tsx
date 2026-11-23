import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { inDesktopApp } from "./data/daemon";
import { startMockDaemon } from "./data/store";

// Inside the desktop app the data comes from tetherd. In a plain browser
// (`npm run dev`) there is no daemon, so preview data stands in for it.
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
