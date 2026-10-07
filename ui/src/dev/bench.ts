// Automated benchmark for SPEC §12.0: drives each scenario in both views, on the
// Changes tab (the whole changeset as one column), and returns one row per metric
// (p95 against the threshold).

import type { EditorView } from "@codemirror/view";
import type { ReviewController as Shell } from "../features/review";
import { KEYS } from "../lib/keymap";
import { nextPaint } from "../lib/nextPaint";
import { changeset } from "../state/changeset.svelte";
import { type Row, percentile95, row } from "./metrics";
import { toggling, tooltipLatencies, views } from "./probe.svelte";

export interface BenchOptions {
  /** Seconds the full-changeset scroll takes (the speed follows from the height). */
  scrollSeconds: number;
  samples: number;
  topology: string;
  memoryMb(): Promise<number | null>;
  progress(message: string): void;
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

async function time(action: () => void | Promise<void>): Promise<number> {
  const t0 = performance.now();
  await action();
  await nextPaint();
  return performance.now() - t0;
}

/** The display's frame interval: the median rAF delta over about one idle second. */
async function refreshInterval(): Promise<number> {
  const deltas: number[] = [];
  await new Promise<void>((resolve) => {
    let last = 0;
    const start = performance.now();
    const frame = (now: number) => {
      if (last) deltas.push(now - last);
      last = now;
      if (performance.now() - start < 1000) requestAnimationFrame(frame);
      else resolve();
    };
    requestAnimationFrame(frame);
  });
  return deltas.sort((a, b) => a - b)[deltas.length >> 1];
}

/** Returns the dropped-frame ratio in percent. */
async function scrollThrough(shell: Shell, root: HTMLElement, seconds: number, interval: number): Promise<number> {
  const view = shell.changes;
  root.scrollTop = 0;
  view.sync();
  await nextPaint();
  const distance = changesProbe().totalHeight() - root.clientHeight;
  const deltas: number[] = [];
  await new Promise<void>((resolve) => {
    let start = 0;
    let last = 0;
    const frame = (now: number) => {
      if (!start) {
        start = last = now;
      } else {
        deltas.push(now - last);
        last = now;
      }
      const progress = Math.min(1, (now - start) / (seconds * 1000));
      root.scrollTop = distance * progress;
      if (progress < 1) requestAnimationFrame(frame);
      else resolve();
    };
    requestAnimationFrame(frame);
  });
  // A frame is dropped when it took over 1.5 refresh intervals (measured idle, before the scroll).
  return (deltas.filter((d) => d > 1.5 * interval).length / deltas.length) * 100;
}

function heapMb(): number | null {
  const memory = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
  return memory ? memory.usedJSHeapSize / 1048576 : null;
}

const info = (metric: string, value: number | null, unit: string, n = 1): Row => ({
  metric,
  p95: value,
  unit,
  threshold: 0,
  cmp: "<",
  n,
  pass: null,
  note: "diagnostic",
});
const check = (metric: string, pass: boolean, note?: string): Row => ({
  metric,
  p95: null,
  unit: "",
  threshold: 0,
  cmp: "<",
  n: 1,
  pass,
  note,
});

/** What the Changes tab's current view lets the benchmark read. */
function changesProbe() {
  const probe = views.changes;
  if (!probe) throw new Error("the Changes view has no probe: the benchmark needs the dev probe on the controller");
  return probe;
}

/** The index of the section with the most target lines. */
const biggestFile = (sections: readonly { file: { targetLines: number } }[]): number =>
  sections.reduce((best, s, i) => (s.file.targetLines > sections[best].file.targetLines ? i : best), 0);

/** The keydown that a binding's spec (`Ctrl-Alt-ArrowDown`, `Space`, `Home`) matches. */
function keydownOf(spec: string): KeyboardEventInit {
  const parts = spec.split("-");
  const name = parts.at(-1) ?? "";
  const mods = parts.slice(0, -1);
  return {
    key: name === "Space" ? " " : name,
    code: name.length === 1 ? `Key${name.toUpperCase()}` : name,
    ctrlKey: mods.includes("Ctrl"),
    altKey: mods.includes("Alt"),
    shiftKey: mods.includes("Shift"),
    metaKey: mods.includes("Meta"),
  };
}

/** Dispatches the key as the app's handlers receive it. */
const press = (target: EventTarget, spec: string) =>
  target.dispatchEvent(
    new KeyboardEvent("keydown", { bubbles: true, cancelable: true, composed: true, ...keydownOf(spec) }),
  );

/** Whether press `i` of a run goes forward: forward and back alternate every `block` presses, and a run turns round at either end. */
const goesForward = (i: number, block: number, atStart: boolean, atEnd: boolean): boolean =>
  (Math.floor(i / block) & 1) === 0 ? !atEnd : atStart;

/** A run of one key pair that moves something in the Changes view. */
interface KeyRun {
  /** Where the key is dispatched. */
  target(): EventTarget;
  /** The key of press `i`, chosen from where the run stands. */
  key(i: number): string;
  /** What the key must change, read before and after each press. */
  state(): unknown;
}

const SYNTHETIC = "synthetic KeyboardEvent, not a trusted key event";

/**
 * Presses the run's keys one at a time, each starting at a random point in a frame.
 * "Keystroke → paint < 16 ms" is read as: the keystroke is painted in the next frame.
 * A sample misses when the next frame starts later than 1.5 refresh intervals after
 * the key, or producing it takes longer than 1.5. A press that left `state()` as it
 * was counts as dead; a run with a dead press measured nothing.
 */
async function keySamples(run: KeyRun, o: BenchOptions, interval: number) {
  const paints: number[] = [];
  let late = 0;
  let dead = 0;
  for (let i = 0; i < o.samples; i++) {
    await sleep(Math.random() * interval);
    const key = run.key(i);
    const before = run.state();
    const t0 = performance.now();
    press(run.target(), key);
    const frameStart = await nextPaint();
    const done = performance.now();
    paints.push(done - t0);
    if (frameStart - t0 > 1.5 * interval || done - frameStart > 1.5 * interval) late++;
    if (run.state() === before) dead++;
  }
  return { paints, late, dead };
}

function keyRows(what: string, label: string, { paints, late, dead }: Awaited<ReturnType<typeof keySamples>>): Row[] {
  const n = paints.length;
  if (dead)
    return [check(`Keystroke → paint, ${what}, ${label}`, false, `${dead} of ${n} key presses changed nothing`)];
  const share = (late / n) * 100;
  return [
    {
      metric: `Keystroke not painted in the next frame, share of samples, ${what}, ${label}`,
      p95: share,
      unit: "%",
      threshold: 5,
      cmp: "<=",
      n,
      pass: share <= 5,
      note: SYNTHETIC,
    },
    info(`Keystroke → paint incl. frame wait, p95, ${what}, ${label}`, percentile95(paints), "ms", n),
  ];
}

/** Puts the caret in the middle of a line near the caret that has text on both sides of it; false if there is none. */
function caretIntoLine(editor: EditorView): boolean {
  const { doc, selection } = editor.state;
  const first = doc.lineAt(selection.main.head).number;
  for (let n = first; n <= Math.min(doc.lines, first + 100); n++) {
    const line = doc.line(n);
    const text = line.text.trim();
    if (text.length < 6) continue;
    const indent = line.text.length - line.text.trimStart().length;
    editor.dispatch({ selection: { anchor: line.from + indent + (text.length >> 1) }, scrollIntoView: true });
    return true;
  }
  return false;
}

/**
 * Keydown → next paint for the keys a reviewer navigates with, on the biggest file: the caret
 * keys in its editor, then next/previous hunk through the window's key handler.
 */
async function navigationKeys(shell: Shell, o: BenchOptions, interval: number, tag: string): Promise<Row[]> {
  const view = shell.changes;
  const probe = changesProbe();
  const files = probe.sections;
  const biggest = biggestFile(files);
  const label = `${files[biggest].file.targetLines}-line file ${tag}`;
  const middle = probe.firstHunkOf(biggest) + (files[biggest].file.hunks.length >> 1);
  await view.goToFile(biggest);

  // The editor is looked up per press: the section unmounts when it leaves the window.
  const editor = (): EditorView => {
    const handle = files[biggest].handle;
    if (!handle) throw new Error("the biggest file's editor was unmounted during the keystroke run");
    return handle.view;
  };
  const caret = (forward: string, back: string, block: number): KeyRun => ({
    target: () => editor().contentDOM,
    key: (i) => {
      const { head } = editor().state.selection.main;
      return goesForward(i, block, head === 0, head === editor().state.doc.length) ? forward : back;
    },
    state: () => editor().state.selection.main.head,
  });

  const rows: Row[] = [];
  const caretRun = async (what: string, run: KeyRun) => {
    await view.goToHunk(middle);
    editor().focus();
    rows.push(...keyRows(what, label, await keySamples(run, o, interval)));
  };
  await caretRun("ArrowDown/ArrowUp", caret("ArrowDown", "ArrowUp", 1));
  await caretRun("PageDown/PageUp", caret("PageDown", "PageUp", 25));

  await view.goToHunk(middle);
  if (caretIntoLine(editor())) {
    editor().focus();
    rows.push(...keyRows("End/Home", label, await keySamples(caret("End", "Home", 1), o, interval)));
  } else rows.push(check(`Keystroke → paint, End/Home, ${label}`, false, "no line with text near the middle hunk"));

  await view.goToHunk(middle);
  editor().focus();
  const hunks: KeyRun = {
    target: () => document.activeElement ?? document.body,
    key: (i) =>
      goesForward(i, 20, view.current === 0, view.current === probe.stopCount - 1)
        ? KEYS.nextHunk.spec
        : KEYS.previousHunk.spec,
    state: () => view.current,
  };
  rows.push(
    ...keyRows(
      `${KEYS.nextHunk.label} / ${KEYS.previousHunk.label}`,
      `${probe.stopCount}-stop changeset ${tag}`,
      await keySamples(hunks, o, interval),
    ),
  );
  rows.push(...(await spaceToggles(shell, o, tag, biggest)));
  return rows;
}

/**
 * Space with the editor focused and the caret in the hunk, as the window handler takes it. The backend
 * answers every toggle, so this is a round trip plus a repaint, not a frame budget: both are
 * diagnostic. Every press must flip the hunk's state and redraw the file's count, and the run
 * leaves the hunk as it found it.
 */
async function spaceToggles(shell: Shell, o: BenchOptions, tag: string, biggest: number): Promise<Row[]> {
  const view = shell.changes;
  const probe = changesProbe();
  const section = probe.sections[biggest];
  const label = `${section.file.targetLines}-line file ${tag}`;
  const hunk = section.review.hunks.findIndex((state) => state !== "seenAgainstOldBase");
  if (hunk < 0)
    return [check(`Space toggles the current hunk ${tag}`, false, "no hunk that toggles back to its state")];
  await view.goToHunk(probe.firstHunkOf(biggest) + hunk);
  // Space with the editor focused and the caret on the hunk's first line, as a reader presses it.
  const editor = (): EditorView => {
    const handle = section.handle;
    if (!handle) throw new Error("the biggest file's editor was unmounted during the Space run");
    return handle.view;
  };
  const { doc } = editor().state;
  const line = Math.min(section.file.hunks[hunk].targetStart + 1, doc.lines);
  editor().dispatch({ selection: { anchor: doc.line(line).from } });
  editor().focus();

  const initial = section.review.hunks[hunk];
  const toRender: number[] = [];
  const toReply: number[] = [];
  let flipped = true;
  const presses = Math.min(o.samples, 50) & ~1; // an even number leaves the hunk as it was
  for (let i = 0; i < presses && flipped; i++) {
    const [state, count] = [section.review.hunks[hunk], section.count.textContent];
    toggling.pending = Promise.resolve();
    const t0 = performance.now();
    press(editor().contentDOM, KEYS.toggleCurrent.spec);
    await toggling.pending;
    toReply.push(performance.now() - t0);
    await nextPaint();
    toRender.push(performance.now() - t0);
    flipped = section.review.hunks[hunk] !== state && section.count.textContent !== count;
  }
  if (section.review.hunks[hunk] !== initial) await view.toggleCurrent();
  const restored = section.review.hunks[hunk] === initial;

  return [
    flipped && restored
      ? check(`Space toggles the current hunk and back ${tag}`, true)
      : check(
          `Space toggles the current hunk and back ${tag}`,
          false,
          flipped ? "the hunk could not be restored" : "a press did not flip the hunk's state and its file's count",
        ),
    ...(flipped
      ? [
          info(`Space (keydown → IPC → repaint), p95, ${label}`, percentile95(toRender), "ms", toRender.length),
          info(`Space (keydown → IPC reply applied), p95, ${label}`, percentile95(toReply), "ms", toReply.length),
        ]
      : []),
  ];
}

/**
 * The two operations the tab model added, both diagnostic: expanding a collapsed
 * unchanged region in the Changes tab, and opening the biggest file in its own tab.
 */
async function expandAndOpen(shell: Shell, tag: string): Promise<Row[]> {
  const view = shell.changes;
  const files = changesProbe().sections;
  const biggest = biggestFile(files);
  const label = `${files[biggest].file.targetLines}-line file ${tag}`;
  const rows: Row[] = [];

  const bars = [...(await view.goToFile(biggest)).view.dom.querySelectorAll<HTMLElement>(".cm-collapsedLines")];
  const expands: number[] = [];
  for (const bar of bars.slice(0, 5)) expands.push(await time(() => bar.click()));
  rows.push(
    expands.length
      ? info(`Expand collapsed region → painted, p95, ${label}`, percentile95(expands), "ms", expands.length)
      : check(`Expand collapsed region ${tag}`, false, "no collapsed region in the biggest file"),
  );

  const path = files[biggest].file.path;
  const opens: number[] = [];
  for (let i = 0; i < 5; i++) {
    opens.push(await time(() => shell.openFile(path, 0)));
    shell.tabs.close(`file:${path}`);
    await nextPaint();
  }
  shell.tabs.active = "changes";
  rows.push(info(`Open file tab → first hunk painted, p95, ${label}`, percentile95(opens), "ms", opens.length));
  return rows;
}

async function mode(
  shell: Shell,
  root: HTMLElement,
  m: "unified" | "split",
  o: BenchOptions,
  interval: number,
): Promise<Row[]> {
  const tag = `[${m}]`;
  const rows: Row[] = [];
  shell.mode = m;

  o.progress(`${tag} open`);
  const opens: number[] = [];
  for (let i = 0; i < 10; i++) opens.push(await shell.open(false));
  rows.push(row(`Open changeset → first hunk painted ${tag}`, opens, "ms", 300));
  await shell.open(true);
  const view = shell.changes;
  const probe = changesProbe();
  const files = probe.sections;

  o.progress(`${tag} hunk navigation`);
  const nav: number[] = [];
  let g = 0;
  for (let i = 0; i < o.samples; i++) {
    g = (g + (i < o.samples / 2 ? 1 : 53)) % view.totalHunks;
    nav.push(await time(() => view.goToHunk(g)));
  }
  rows.push(row(`Hunk-to-hunk navigation → painted ${tag}`, nav, "ms", 50));

  o.progress(`${tag} keystroke`);
  rows.push(...(await navigationKeys(shell, o, interval, tag)));

  o.progress(`${tag} expand and file tab`);
  rows.push(...(await expandAndOpen(shell, tag)));

  o.progress(`${tag} toggle Seen`);
  const near1000 = files.reduce(
    (best, s, i) => (Math.abs(s.file.targetLines - 1000) < Math.abs(files[best].file.targetLines - 1000) ? i : best),
    0,
  );
  await view.goToFile(near1000);
  await nextPaint();
  const button = probe.seenButton(near1000);
  const toggles: number[] = [];
  // Click, IPC round trip to the backend, repaint of the returned state.
  for (let i = 0; i < Math.min(o.samples, 50); i++) {
    toggles.push(
      await time(async () => {
        button.click();
        await toggling.pending;
      }),
    );
  }
  rows.push(
    row(
      `Toggle Seen (click → IPC → repaint), ${files[near1000].file.targetLines}-line file, ${files[near1000].file.hunks.length} hunks ${tag}`,
      toggles,
      "ms",
      50,
    ),
  );

  o.progress(`${tag} scroll (${o.scrollSeconds}s)`);
  const dropped = await scrollThrough(shell, root, o.scrollSeconds, interval);
  rows.push(
    row(`Sustained scroll, dropped frames (refresh ${interval.toFixed(1)} ms) ${tag}`, [dropped], "%", 5, "<="),
  );
  rows.push(info(`Section height estimate vs measured, max error px ${tag}`, probe.maxDrift(), "px"));
  rows.push(info(`Merge chunks ≠ backend hunks, mounted files ${tag}`, probe.chunkMismatches(), "files"));

  const [heap, rss] = [heapMb(), await o.memoryMb()];
  rows.push(info(`Renderer JS heap ${tag}`, heap, "MB"));
  rows.push(
    rss === null
      ? {
          metric: `Renderer memory, full changeset open ${tag}`,
          p95: null,
          unit: "MB",
          threshold: 1024,
          cmp: "<",
          n: 0,
          pass: null,
          note: "the host cannot read this webview's process memory; use Activity Monitor / Task Manager",
        }
      : row(
          `Renderer memory, webview process tree, full changeset open ${tag}`,
          [rss],
          "MB",
          1024,
          "<",
          "summed over processes; shared pages count per process",
        ),
  );
  return rows;
}

/** Hovers real lines with synthetic mouse moves and times query → painted tooltip. */
async function hoverToTooltip(shell: Shell, o: BenchOptions): Promise<number[]> {
  const editor = (await shell.changes.goToFile(0)).view;
  const lines = [...editor.contentDOM.querySelectorAll<HTMLElement>(".cm-line")].filter((l) => l.textContent);
  tooltipLatencies.length = 0;
  const move = (x: number, y: number) =>
    editor.contentDOM.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y, bubbles: true }));
  for (let i = 0; i < Math.min(30, lines.length * 2); i++) {
    const rect = lines[i % lines.length].getBoundingClientRect();
    move(rect.left + 3, rect.top - 40); // away from the previous tooltip
    await sleep(80);
    const before = tooltipLatencies.length;
    move(rect.left + 12 + ((i * 11) % 60), rect.top + rect.height / 2);
    for (let waited = 0; tooltipLatencies.length === before && waited < 2000; waited += 20) await sleep(20);
  }
  o.progress(`hover: ${tooltipLatencies.length} tooltips`);
  return [...tooltipLatencies];
}

export async function runBenchmark(shell: Shell, root: HTMLElement, o: BenchOptions): Promise<Row[]> {
  shell.tabs.active = "changes"; // the benchmark measures the Changes tab
  const interval = await refreshInterval();
  const rows: Row[] = [];
  rows.push(...(await mode(shell, root, "unified", o, interval)));
  rows.push(...(await mode(shell, root, "split", o, interval)));

  const topology = `topology: ${o.topology}`;
  const note = o.topology === "wsl-split" ? undefined : "native topology; §12.0 requires the WSL split";
  o.progress("hover round trip");
  const hover: number[] = [];
  const path = shell.files[0].path;
  for (let i = 0; i < 200; i++) {
    const t0 = performance.now();
    await shell.backend.hover(changeset.manifest?.revision ?? 0, "target", path, i % 10, 0);
    hover.push(performance.now() - t0);
  }
  rows.push(row(`Hover round trip, request → response (${topology})`, hover, "ms", 100, "<", note));
  const tooltips = await hoverToTooltip(shell, o);
  rows.push(
    tooltips.length >= 10
      ? row(`Hover → tooltip painted (${topology})`, tooltips, "ms", 100, "<", note)
      : check(`Hover → tooltip painted (${topology})`, false, `only ${tooltips.length} tooltips appeared`),
  );
  return rows;
}
