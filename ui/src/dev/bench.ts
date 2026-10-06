// Automated benchmark for SPEC §12.0: drives each scenario in both views and
// returns one row per metric (p95 against the threshold).

import type { ReviewController as Shell } from "../features/review";
import { nextPaint } from "../lib/nextPaint";
import { type Row, percentile95, row } from "./metrics";

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
  const view = shell.view!;
  root.scrollTop = 0;
  view.sync();
  await nextPaint();
  const distance = view.totalHeight - root.clientHeight;
  const deltas: number[] = [];
  await new Promise<void>((resolve) => {
    let start = 0;
    let last = 0;
    const frame = (now: number) => {
      if (!start) start = last = now;
      else (deltas.push(now - last), (last = now));
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

/** Start time of the next frame and the time its paint completed. */
function framePaint(): Promise<{ frameStart: number; done: number }> {
  return new Promise((resolve) =>
    requestAnimationFrame((frameStart) => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => resolve({ frameStart, done: performance.now() });
      channel.port2.postMessage(null);
    }),
  );
}

/**
 * Types one character at a time through the DOM input path: an `insertText`
 * edit command on the focused editor fires `beforeinput`/`input` and the browser
 * edits the contenteditable, which CodeMirror reads back, as for a real key. (A
 * trusted key event cannot be synthesized.) Each sample starts at a random point
 * in a frame. "Keystroke → paint < 16 ms" is read as: the keystroke is painted
 * in the next frame. A sample misses when the next frame starts later than 1.5
 * refresh intervals after the input, or producing it takes longer than 1.5.
 */
async function typeSamples(shell: Shell, o: BenchOptions, interval: number): Promise<{ rows: Row[]; tag: string }> {
  const tag = `[${shell.mode}]`;
  const view = shell.view!;
  const files = view.sections;
  const biggest = files.reduce((best, s, i) => (s.file.targetLines > files[best].file.targetLines ? i : best), 0);
  const big = await view.goToFile(biggest);
  await view.goToHunk(view.firstHunkOf(biggest) + (files[biggest].file.hunks.length >> 1));
  const editor = big.view;
  editor.focus();
  const [at, length, decorations] = [editor.state.selection.main.head, editor.state.doc.length, big.decorationCount()];
  const keys: number[] = [];
  const handling: number[] = [];
  let missed = 0;
  for (let i = 0; i < o.samples; i++) {
    await sleep(Math.random() * interval);
    const t0 = performance.now();
    document.execCommand("insertText", false, "x");
    await Promise.resolve(); // CodeMirror's DOM observer runs in a microtask
    handling.push(performance.now() - t0);
    const { frameStart, done } = await framePaint();
    keys.push(done - t0);
    if (frameStart - t0 > 1.5 * interval || done - frameStart > 1.5 * interval) missed++;
  }
  const typed = editor.state.doc.length - length;
  const intact = big.decorationCount() === decorations; // typing inside a line must not add or drop decorations
  editor.dispatch({ changes: { from: at, to: at + typed, insert: "" } });
  const label = `${files[biggest].file.targetLines}-line file ${tag}`;
  return {
    tag,
    rows:
      typed === o.samples
        ? [
            {
              metric: `Keystroke not painted in the next frame, share of samples, ${label}`,
              p95: (missed / o.samples) * 100,
              unit: "%",
              threshold: 5,
              cmp: "<=",
              n: o.samples,
              pass: (missed / o.samples) * 100 <= 5,
              note: "synthetic insertText, not a trusted key event",
            },
            info(`Keystroke → paint incl. frame wait, p95, ${label}`, percentile95(keys), "ms", keys.length),
            info(
              `Keystroke input handling (insertText → CodeMirror update), p95, ${label}`,
              percentile95(handling),
              "ms",
              handling.length,
            ),
            check(`Decorations intact after typing ${tag}`, intact),
          ]
        : [
            check(
              `Keystroke → paint via DOM input ${tag}`,
              false,
              `insertText inserted ${typed} of ${o.samples} characters`,
            ),
          ],
  };
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
  const view = shell.view!;
  const files = view.sections;

  o.progress(`${tag} hunk navigation`);
  const nav: number[] = [];
  let g = 0;
  for (let i = 0; i < o.samples; i++) {
    g = (g + (i < o.samples / 2 ? 1 : 53)) % view.totalHunks;
    nav.push(await time(() => view.goToHunk(g)));
  }
  rows.push(row(`Hunk-to-hunk navigation → painted ${tag}`, nav, "ms", 50));

  o.progress(`${tag} keystroke`);
  rows.push(...(await typeSamples(shell, o, interval)).rows);

  o.progress(`${tag} toggle Seen`);
  const near1000 = files.reduce(
    (best, s, i) => (Math.abs(s.file.targetLines - 1000) < Math.abs(files[best].file.targetLines - 1000) ? i : best),
    0,
  );
  await view.goToFile(near1000);
  await nextPaint();
  const button = view.seenButton(near1000);
  const toggles: number[] = [];
  // Click, IPC round trip to the backend, repaint of the returned state.
  for (let i = 0; i < Math.min(o.samples, 50); i++) {
    toggles.push(
      await time(async () => {
        button.click();
        await view.pendingToggle;
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
  rows.push(info(`Layout drift, max px ${tag}`, view.maxDrift(), "px"));
  rows.push(info(`Merge chunks ≠ backend hunks, mounted files ${tag}`, view.chunkMismatches(), "files"));

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
  const view = shell.view!;
  await view.goToFile(0);
  const editor = view.handleOf(0)!.view;
  const lines = [...editor.contentDOM.querySelectorAll<HTMLElement>(".cm-line")].filter((l) => l.textContent);
  shell.tooltipLatencies.length = 0;
  const move = (x: number, y: number) =>
    editor.contentDOM.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y, bubbles: true }));
  for (let i = 0; i < Math.min(30, lines.length * 2); i++) {
    const rect = lines[i % lines.length].getBoundingClientRect();
    move(rect.left + 3, rect.top - 40); // away from the previous tooltip
    await sleep(80);
    const before = shell.tooltipLatencies.length;
    move(rect.left + 12 + ((i * 11) % 60), rect.top + rect.height / 2);
    for (let waited = 0; shell.tooltipLatencies.length === before && waited < 2000; waited += 20) await sleep(20);
  }
  o.progress(`hover: ${shell.tooltipLatencies.length} tooltips`);
  return [...shell.tooltipLatencies];
}

export async function runBenchmark(shell: Shell, root: HTMLElement, o: BenchOptions): Promise<Row[]> {
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
    await shell.backend.hover(path, i % 10, 0);
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
