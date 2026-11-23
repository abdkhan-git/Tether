import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { startMockDaemon } from "./data/store";

// Preview data stands in for the daemon until the Tauri IPC client exists.
startMockDaemon();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
