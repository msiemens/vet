import { mount } from "svelte";
import "./lib/theme/tokens.css";
import "./lib/theme/global.css";
import { Splash } from "./features/splash";

const target = document.getElementById("app");
if (!target) throw new Error("splash.html has no #app element");
mount(Splash, { target });
