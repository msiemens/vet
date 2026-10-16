// What a scripted session runs on: a scratch repository, a real backend with a scratch data
// directory, the UI's own stores connected to it, and the means to wait until a step has settled.

import type { View } from "../../src/generated/View.ts";
import type { Request } from "../../src/generated/Request.ts";
import type { Shown } from "../../src/state/shown.ts";
import { Backend } from "../../src/state/backend.ts";
import { opened } from "../../src/state/opened.svelte.ts";
import { notices } from "../../src/state/notices.svelte.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { reviewShown } from "../../src/state/reviewShown.svelte.ts";
import { session } from "../../src/state/session.svelte.ts";
import "../../src/state/platform.svelte.ts";
import { BackendProcess, Fixture, binary } from "./fixture.ts";
import { checkInvariants, type Violation } from "./invariants.ts";
import { onView } from "./onView.ts";
import { type SessionName, startOnQueue } from "./sessionNames.ts";

const REQUESTS_THAT_NAME_A_VIEW = new Set<Request["type"]>([
  "toggleHunk",
  "toggleFlag",
  "toggleItem",
  "toggleFileSeen",
  "setFilesSeen",
  "commentTarget",
  "createThread",
]);

/** The requests that change review state or add comments: a step that does not name them sends none. */
const MARKS = new Set<Request["type"]>([
  "toggleHunk",
  "toggleFlag",
  "toggleItem",
  "toggleFileSeen",
  "setFilesSeen",
  "createThread",
]);

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** How many requests of each kind a step sent. A manifest is told by its view: `manifest(commit)`. */
export type RequestMap = Record<string, number>;

const kindOf = (r: Request) => (r.type === "manifest" ? `manifest(${r.view.type})` : r.type);

/** The kinds of `requests` with their counts; `waitForQuiet`, which the harness itself sends, is left out. */
function requestMap(requests: readonly Request[]): RequestMap {
  const map: RequestMap = {};
  for (const r of requests) if (r.type !== "waitForQuiet") map[kindOf(r)] = (map[kindOf(r)] ?? 0) + 1;
  return map;
}

export interface Harness {
  readonly fixture: Fixture;
  /** A second connection to the same project: another window, and the source of fresh reads. */
  readonly other: Backend;
  /** The requests the window sent, in order. */
  readonly sent: Request[];
  /**
   * Runs `run`, waits until the window has settled, then compares the stores with the backend. The step
   * also fails unless the backend answered every request, the window's notice is `notice` (default none)
   * and the backend rejected `rejected` requests (default none). It fails unless the window sent
   * `sent`: how many requests of each kind (a manifest is told by its view, `manifest(commit)`). Kinds
   * `sent` leaves out are free, except the review requests, which are none unless named; with `exact`
   * no other kind is sent at all.
   */
  step(
    name: string,
    run: () => unknown,
    expected?: {
      sent?: RequestMap;
      exact?: boolean;
      notice?: string;
      rejected?: number;
    },
  ): Promise<void>;
  /** Resolves when `condition` holds; fails after a while. */
  until(condition: () => boolean, what: string): Promise<void>;
  /** The view the elements on screen were built for: what the manifest on view is of. */
  shown(): Shown;
  /**
   * Runs `send`, which makes a review or comment request for the element built for `from` (default:
   * what is on screen), and records a request that names another view.
   */
  asElement<T>(send: () => T, from?: Shown): T;
  /** Lets the process end the window's socket; resolves when the window has reconnected. */
  dropSocket(): Promise<void>;
}

export interface StepResult {
  name: string;
  violations: Violation[];
}

/** Runs `script` against a fresh backend and fixture; resolves with what each step found. */
export async function runSession(name: SessionName, script: (h: Harness) => Promise<void>): Promise<StepResult[]> {
  const fixture = new Fixture();
  const backend = new BackendProcess(binary(), fixture.home);
  const results: StepResult[] = [];
  try {
    const { port, token } = await backend.address;
    globalThis.location = new URL(
      `http://localhost/#port=${port}&token=${token}&repo=${encodeURIComponent(fixture.repo)}`,
    ) as never;
    await session.connect();
    if (!session.backend || session.repo === undefined)
      throw new Error(`the window did not connect: ${session.status}`);
    const app = session.backend;
    const other = await Backend.connect();
    await other.open(fixture.repo);

    const sent: Request[] = [];
    const wrongViews: Violation[] = [];
    const failed: string[] = [];
    let element: View | undefined;
    let inFlight = 0;
    const conn = (
      app as unknown as {
        conn: { request(r: Request, s?: AbortSignal): Promise<unknown>; onEvent(l: () => void): void };
      }
    ).conn;
    const request = conn.request.bind(conn);
    conn.request = (r, s) => {
      sent.push(r);
      if (
        element &&
        REQUESTS_THAT_NAME_A_VIEW.has(r.type) &&
        "view" in r &&
        JSON.stringify(r.view) !== JSON.stringify(element)
      ) {
        wrongViews.push({
          invariant: "requestView",
          view: element.type,
          what: `${r.type} of an element built for ${JSON.stringify(element)}`,
          ui: r.view,
          backend: element,
        });
      }
      inFlight++;
      const answer = request(r, s);
      // A request the window itself gave up on is not a failure.
      answer.catch((error: unknown) => {
        if (!s?.aborted) failed.push(`${r.type}: ${error instanceof Error ? error.message : String(error)}`);
      });
      return answer.finally(() => {
        inFlight--;
      });
    };

    await (await opened.current())?.refresh();
    await queue.loadAll();

    // Every session but the queue's shows the review screen: its stores follow it, as the window does.
    if (!startOnQueue.has(name)) reviewShown.set(true);

    /** No request in flight and no background work left in the backend or the window. */
    const idle = async () => {
      const start = Date.now();
      for (;;) {
        while (inFlight > 0) {
          if (Date.now() - start > 30_000) throw new Error("the window did not settle");
          await sleep(20);
        }
        // The answer is ordered after every message the backend sent this connection before it, so
        // the stores have seen everything the backend did.
        await app.waitForQuiet();
        // The window reacts to those messages with reads of its own, some debounced.
        if (inFlight === 0 && !queue.settling && !opened.session?.settling) return;
        if (Date.now() - start > 30_000) throw new Error("the window did not settle");
      }
    };

    const h: Harness = {
      fixture,
      other,
      sent,
      async step(name, run, expected = {}) {
        const before = sent.length;
        failed.length = 0;
        notices.close();
        await run();
        // What another window did reaches this one as events after `run` ended.
        await idle();
        const violations = [...wrongViews.splice(0), ...(await checkInvariants(other))];
        const step = (what: string, ui: unknown, backend: unknown) =>
          violations.push({ invariant: "step", view: onView().view.type, what, ui, backend });
        const got = requestMap(sent.slice(before));
        const want = expected.sent ?? {};
        const kinds = new Set([...Object.keys(want), ...Object.keys(got)]);
        const checked = (kind: string) => expected.exact === true || (MARKS as Set<string>).has(kind) || kind in want;
        const wrong = [...kinds].filter((kind) => checked(kind) && (got[kind] ?? 0) !== (want[kind] ?? 0));
        if (wrong.length) step("the requests the step sent", got, want);
        if (failed.length !== (expected.rejected ?? 0)) step("requests the backend rejected", failed.slice(), []);
        if (notices.text !== expected.notice) step("the notice after the step", notices.text, expected.notice);
        results.push({ name, violations });
        process.stderr.write(`  ${name}: ${violations.length ? `${violations.length} differences` : "ok"}\n`);
      },
      async until(condition, what) {
        const start = Date.now();
        while (!condition()) {
          if (Date.now() - start > 20_000) throw new Error(`timed out waiting for ${what}`);
          await sleep(25);
        }
      },
      shown() {
        const shown = opened.session?.currentShown();
        if (!shown) throw new Error("no changeset is open");
        return shown;
      },
      asElement(send, from = h.shown()) {
        element = from.view;
        try {
          return send();
        } finally {
          element = undefined;
        }
      },
      async dropSocket() {
        const back = new Promise<void>((resolve) => app.onReconnect(resolve));
        (app as unknown as { conn: { socket: { close(): void } } }).conn.socket.close();
        await back;
      },
    };
    await script(h);
    return results;
  } finally {
    backend.stop();
    fixture.remove();
  }
}
