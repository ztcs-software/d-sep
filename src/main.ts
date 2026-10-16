import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

// Feel like an app, not a web page: no "Reload / Inspect" menu in release builds. Text fields keep their
// Cut / Copy / Paste menu.
if (!import.meta.env.DEV) {
  document.addEventListener("contextmenu", (event) => {
    if (!(event.target instanceof Element && event.target.closest("input, textarea"))) event.preventDefault();
  });
}

mount(App, { target: document.getElementById("app")! });
