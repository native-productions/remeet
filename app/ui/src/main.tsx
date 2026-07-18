import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { MainApp } from "./main-window/MainApp";
import { PopoverApp } from "./popover/PopoverApp";
import "./styles.css";
import "./main-window/window.css";

// One bundle serves both windows. Which shell renders is decided by the Tauri
// window label, so the two surfaces share components, tokens, and IPC types
// without a router or a second build.
const label = getCurrentWindow().label;
document.body.dataset.window = label;

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>{label === "popover" ? <PopoverApp /> : <MainApp />}</StrictMode>,
  );
}
