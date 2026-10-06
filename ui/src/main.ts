import { mount } from "svelte";
import "./lib/theme/tokens.css";
import "./lib/theme/global.css";
import App from "./app/App.svelte";

mount(App, { target: document.getElementById("app")! });
