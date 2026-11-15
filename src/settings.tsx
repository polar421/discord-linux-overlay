import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { Settings } from "./settings/Settings";
import "./styles.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("settings root element is missing");
}

createRoot(container).render(
  <StrictMode>
    <Settings />
  </StrictMode>,
);
