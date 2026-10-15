// The scratch repository a session reviews, and the real backend process it runs against. Kept free
// of the stores so the browser tier can use them under Playwright without the rune loader.

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

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
    // The backend's fetch writes reflog entries, which need an identity from the repository's configuration.
    this.git(this.repo, "config", "user.name", "t");
    this.git(this.repo, "config", "user.email", "t@t");
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
    this.git(this.repo, "push", "-q", "-u", "origin", "feature");
    this.write("a.txt", "wt", [2, 10, 18, 30]);
    this.write("b.txt", "wt", [7, 25]);
    this.write("src/c.txt", "wt", [12, 20]);
  }

  /** A 40-line file of which the lines `edited` differ from the base text. */
  write(path: string, tag: string, edited: number[]) {
    const lines = Array.from({ length: 40 }, (_, i) => (edited.includes(i) ? `edited ${i} ${tag}` : `line ${i}`));
    mkdirSync(join(this.repo, path, ".."), { recursive: true });
    writeFileSync(join(this.repo, path), lines.join("\n") + "\n");
  }

  /**
   * Writes `count` new files under `many/`, each a 40-line text file with one line changed, so a
   * changeset of many files has to be scrolled and loaded. The extension is not prose, so the lines
   * do not wrap and a section's height comes from the manifest alone.
   */
  manyFiles(count: number) {
    for (let i = 0; i < count; i++) this.write(`many/f${String(i).padStart(3, "0")}.dat`, `m${i}`, [20]);
  }

  /** Moves `HEAD` to `ref` and leaves the files as they are. */
  resetHeadTo(ref: string) {
    this.git(this.repo, "reset", "-q", "--soft", ref);
  }

  /** Commits the working tree's edits, which makes the branch a changeset of four commits. */
  commitWorkingTree() {
    this.git(this.repo, "commit", "-qam", "c4: the working tree");
  }

  /** The branch on view is pushed to: an edit of `path` committed on `feature`. */
  push(path: string, tag: string, edited: number[]) {
    this.write(path, tag, edited);
    this.git(this.repo, "commit", "-qam", `push: ${path}`);
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
export class BackendProcess {
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

export const binary = () =>
  process.env.VET_BACKEND ??
  join(process.env.CARGO_TARGET_DIR ?? join(import.meta.dirname, "../../../target"), "debug/vet-backend");
