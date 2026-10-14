// Preloaded with `node --import`: installs the hooks and the little of a browser the stores read at
// import time (the harness sets `location` before it connects).

import { registerHooks } from "node:module";
import { load, resolve } from "./loader.mjs";

registerHooks({ resolve, load });

globalThis.window = globalThis;
globalThis.addEventListener = () => {};
