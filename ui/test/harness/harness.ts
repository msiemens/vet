// What a scripted session runs on: a scratch repository, a real backend with a scratch data
// directory, the UI's own stores connected to it, and the means to wait until a step has settled.

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { View } from "../../src/generated/View.ts";
import type { Request } from "../../src/generated/Request.ts";
import type { Shown } from "../../src/state/shown.ts";
import { Backend } from "../../src/state/backend.ts";
import { changeset } from "../../src/state/changeset.svelte.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { session } from "../../src/state/session.svelte.ts";
import { threads } from "../../src/state/threads.svelte.ts";
import { versions } from "../../src/state/versions.svelte.ts";
import { checkInvariants, type Violation, ViewMirror } from "./invariants.ts";

const REQUESTS_THAT_NAME_A_VIEW = new Set<Request["type"]>([
  "toggleHunk",
  "toggleFlag",
  "toggleItem",
  "toggleFileSeen",
  "setFilesSeen",
  "commentTarget",
  "createThread",
]);

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** The repository the sessions review: `feature` has three commits on `main`, and the working tree has a change of its own. */
export class Fixture {
  readonly root = mkdtempSync(join(tmpdir(), "vet-harness-"));
  readonly home = join(this.root, "home");
  readonly repo = join(this.root, "repo");

  constructor() {
    mkdirSync(this.home);
    const origin = join(this.root, "origin.git");
    this.git(this.root, "init", "-q", "--bare", origin);
    this.git(this.root, "init", "-q", this.repo);
    this.write("a.txt", "x", []);
    this.write("b.txt", "x", []);
    this.write("src/c.txt", "x", []);
    this.git(this.repo, "add", ".");
    this.git(this.repo, "commit", "-qm", "base");
    this.git(this.repo, "remote", "add", "origin", origin);
    this.git(this.repo, "push", "-q", "origin", "main");
    this.git(this.repo, "checkout", "-qb", "feature");
    this.write("a.txt", "c1", [2, 30]);
    this.git(this.repo, "commit", "-qam", "c1: a.txt");
    this.write("b.txt", "c2", [5]);
    this.write("src/c.txt", "c2", [12]);
    this.git(this.repo, "commit", "-qam", "c2: b.txt and src/c.txt");
    this.write("a.txt", "c3", [2, 10, 30]);
    this.git(this.repo, "commit", "-qam", "c3: a.txt again");
    this.git(this.repo, "push", "-q", "origin", "feature");
    this.write("b.txt", "wt", [7, 25]);
  }

  /** A 40-line file of which the lines `edited` differ from the base text. */
  write(path: string, tag: string, edited: number[]) {
    const lines = Array.from({ length: 40 }, (_, i) => (edited.includes(i) ? `edited ${i} ${tag}` : `line ${i}`));
    mkdirSync(join(this.repo, path, ".."), { recursive: true });
    writeFileSync(join(this.repo, path), lines.join("\n") + "\n");
  }

  private git(cwd: string, ...args: string[]) {
    const env = {
      ...process.env,
      HOME: this.home,
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_CONFIG_SYSTEM: "/dev/null",
      GIT_AUTHOR_NAME: "t",
      GIT_AUTHOR_EMAIL: "t@t",
      GIT_COMMITTER_NAME: "t",
      GIT_COMMITTER_EMAIL: "t@t",
    };
    execFileSync("git", ["-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, env });
  }

  remove() {
    rmSync(this.root, { recursive: true, force: true });
  }
}

/** A backend process with the fixture's home as its data directory. */
class BackendProcess {
  readonly child: ChildProcess;
  readonly address: Promise<{ port: number; token: string }>;
  private stderr = "";

  constructor(binary: string, home: string) {
    const env = {
      ...process.env,
      HOME: home,
      XDG_DATA_HOME: join(home, "data"),
      XDG_CONFIG_HOME: join(home, "config"),
    };
    // The backend ends with its stdin, so it does not outlive a harness that dies.
    this.child = spawn(binary, ["--exit-on-stdin-close"], { env, stdio: ["pipe", "pipe", "pipe"] });
    this.child.stderr?.on("data", (chunk: Buffer) => (this.stderr += chunk.toString()));
    this.address = new Promise((resolve, reject) => {
      let out = "";
      this.child.stdout?.on("data", (chunk: Buffer) => {
        out += chunk.toString();
        const line = out.split("\n")[0];
        if (out.includes("\n")) resolve(JSON.parse(line) as { port: number; token: string });
      });
      this.child.once("exit", () => reject(new Error(`the backend ended before it was ready:\n${this.stderr}`)));
    });
  }

  stop() {
    this.child.kill();
  }
}

export interface Harness {
  readonly fixture: Fixture;
  /** A second connection to the same project: another window, and the source of fresh reads. */
  readonly other: Backend;
  /** The requests the window sent, in order. */
  readonly sent: Request[];
  /** Runs `run`, waits until the window has settled, then compares the stores with the backend. */
  step(name: string, run: () => unknown): Promise<void>;
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

const binary = () =>
  process.env.VET_BACKEND ??
  join(process.env.CARGO_TARGET_DIR ?? join(import.meta.dirname, "../../../target"), "debug/vet-backend");

/** Runs `script` against a fresh backend and fixture; resolves with what each step found. */
export async function runSession(script: (h: Harness) => Promise<void>): Promise<StepResult[]> {
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
    let element: View | undefined;
    let activity = Date.now();
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
      activity = Date.now();
      return request(r, s).finally(() => {
        inFlight--;
        activity = Date.now();
      });
    };
    conn.onEvent(() => (activity = Date.now()));

    const stopVersions = versions.start(app);
    const stopThreads = threads.start(app);
    const mirror = new ViewMirror();
    await changeset.refresh();
    await queue.loadAll();
    // The window shows the queue, which follows the events for the changeset on view.
    const stopQueue = queue.watch();

    /** No request in flight and nothing heard for a moment; the mirror of the views follows the stores meanwhile. */
    const idle = async () => {
      const start = Date.now();
      for (;;) {
        mirror.follow();
        if (inFlight === 0 && Date.now() - activity > 200) return;
        if (Date.now() - start > 30_000) throw new Error("the window did not settle");
        await sleep(20);
      }
    };

    const h: Harness = {
      fixture,
      other,
      sent,
      async step(name, run) {
        await run();
        // What another window did reaches this one as events after `run` ended.
        activity = Date.now();
        await idle();
        const violations = [...wrongViews.splice(0), ...(await checkInvariants(other, mirror))];
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
        const shown = changeset.currentShown();
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
    try {
      await script(h);
    } finally {
      stopVersions();
      stopThreads();
      stopQueue();
    }
    return results;
  } finally {
    backend.stop();
    fixture.remove();
  }
}
