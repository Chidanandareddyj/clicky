import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import OverlayAppRoot from "./overlay-main";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <OverlayAppRoot />
  </StrictMode>,
);
