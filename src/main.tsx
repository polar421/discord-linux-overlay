import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { isTauri } from "./lib/bridge";
import { Overlay } from "./overlay/Overlay";
import "./styles.css";

// Outside Tauri this page is the browser preview: frame it like the real
// 320x440 overlay window so the layout can be reviewed honestly.
if (!isTauri) {
  document.body.classList.add("browser-preview");
}

const container = document.getElementById("root");
if (!container) {
  throw new Error("overlay root element is missing");
}

createRoot(container).render(
  <StrictMode>
    <Overlay />
  </StrictMode>,
);
