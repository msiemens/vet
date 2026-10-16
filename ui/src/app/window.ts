// The window's wiring that is not a component: the key listener, the double-click setting, the
// connection that reveals the window, the native menu, double Shift, and the Pipelines tool's
// hide for a changeset of another kind. `App` installs it once while it is mounted.

import { watchDoubleShift } from "../features/palette";
import { leaveScope, shell } from "../features/chrome";
import { belongsTo } from "../features/pipelines";
import { setDoubleClickMs } from "../lib/ui/doubleClick";
import { commands } from "../state/commands";
import { appVersion, hostLabel, inTauri, showWindow, systemDoubleClickMs } from "../state/host";
import { mainMenu } from "../state/mainMenu.svelte";
import { opened } from "../state/opened.svelte";
import { session } from "../state/session.svelte";
import { appState } from "./appState.svelte";

export function installWindow(): () => void {
  // Capturing, so a command's key reaches it before any control or editor handles the key.
  const onkeydown = (e: KeyboardEvent) => void commands.dispatch(e);
  window.addEventListener("keydown", onkeydown, true);
  window.addEventListener("keydown", leaveScope);
  // The webview's own menu stays on text fields; elsewhere a handler opened the app's menu, or there is none.
  const onContextMenu = (e: MouseEvent) => {
    if (!(e.target instanceof Element && e.target.closest("input, textarea"))) e.preventDefault();
  };
  window.addEventListener("contextmenu", onContextMenu);
  // The viewer may change the mouse settings while the window stays open.
  const readDoubleClick = () =>
    void systemDoubleClickMs().then(
      (ms) => ms !== undefined && setDoubleClickMs(ms),
      (e: unknown) => console.error(e),
    );
  readDoubleClick();
  window.addEventListener("focus", readDoubleClick);
  // The splash covers the start until the backend is connected (or the connection failed, which
  // the window then shows), so the window never shows empty.
  let shown: Promise<void> | undefined;
  const reveal = () => {
    shown ??= showWindow().then(
      (frame) => {
        if (frame === "native") console.warn("app-drawn title bar unavailable; the host log says why");
      },
      (e: unknown) => {
        console.error(e);
      },
    );
  };
  if (inTauri) void appVersion().then((v) => (appState.version = v));
  void hostLabel().then((label) => (appState.host = label));
  session.onConnect(reveal);
  void session.connect().then(reveal);
  // The Pipelines tool belongs to a pull request: a changeset of another kind closes it.
  const stopPipelines = opened.onSession((next) => {
    if (next && !belongsTo(next.key.type) && shell.bottom === "pipelines") shell.hideBottom();
  });
  const stopMenu = mainMenu.listen();
  const stopDoubleShift = watchDoubleShift(
    () => !!session.repo && !commands.isCapturing(),
    () => commands.run("palette.all"),
  );
  return () => {
    stopPipelines();
    stopMenu();
    stopDoubleShift();
    window.removeEventListener("keydown", onkeydown, true);
    window.removeEventListener("keydown", leaveScope);
    window.removeEventListener("contextmenu", onContextMenu);
    window.removeEventListener("focus", readDoubleClick);
  };
}
