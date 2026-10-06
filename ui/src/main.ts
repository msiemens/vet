import { mount } from "svelte";
import "./lib/theme/tokens.css";
import "./lib/theme/global.css";
import App from "./app/App.svelte";

const target = document.getElementById("app");
if (!target) throw new Error("index.html has no #app element");
mount(App, { target });
