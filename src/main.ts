import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

// Feel like an app, not a web page: no "Reload / Inspect" menu in release builds. Text fields keep their
// Cut / Copy / Paste menu.
if (!import.meta.env.DEV) {
  document.addEventListener("contextmenu", (event) => {
    if (!(event.target instanceof Element && event.target.closest("input, textarea"))) event.preventDefault();
  });
  // Nor F5 or Ctrl+R, which WebView2 still takes as Reload: the page would forget a sort that keeps running behind it,
  // and with it the Cancel button, the result and the Undo.
  document.addEventListener("keydown", (event) => {
    const reload = event.key === "F5" || event.key === "BrowserRefresh" || (event.ctrlKey && event.key.toLowerCase() === "r");
    if (reload) event.preventDefault();
  });
}

mount(App, { target: document.getElementById("app")! });
