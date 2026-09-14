import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import Overlay from "./Overlay";
import "./styles.css";

const isOverlay = new URLSearchParams(window.location.search).get("window") === "overlay";
document.documentElement.dataset.window = isOverlay ? "overlay" : "main";
document.body.dataset.window = isOverlay ? "overlay" : "main";

createRoot(document.getElementById("root")!).render(
  <StrictMode>{isOverlay ? <Overlay /> : <App />}</StrictMode>,
);
