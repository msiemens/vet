// The little of a browser the state modules read at import time, for the browser tier's node side.
// Unlike tier 1 this side keeps no stores, so it needs no rune loader: the driver reads the app
// through the page.

const globals = globalThis as unknown as Record<string, unknown>;
globals.window = globalThis;
globals.addEventListener = () => undefined;
