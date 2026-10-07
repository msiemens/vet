import assert from "node:assert/strict";
import { test } from "node:test";
import { type OpenSteps, RepoOpener } from "./repoOpener.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}

function steps(open: (path: string) => Promise<string>) {
  const log: string[] = [];
  const port: OpenSteps = {
    release: () => (log.push("release"), Promise.resolve()),
    open: (path) => (log.push(`open ${path}`), open(path)),
    setWindowMode: (mode) => (log.push(`mode ${mode}`), Promise.resolve()),
    opened: (root) => log.push(`opened ${root}`),
    failed: (path, error) => log.push(`failed ${path}: ${(error as Error).message}`),
  };
  return { log, port };
}

void test("opening a path twice while it opens sends one open", async () => {
  const session = deferred<string>();
  const { log, port } = steps(() => session.promise);
  const opener = new RepoOpener(port);

  const first = opener.open("/repo");
  const second = opener.open("/repo");
  session.resolve("/repo");
  await Promise.all([first, second]);

  assert.deepEqual(log, ["release", "open /repo", "mode project", "opened /repo"]);
});

void test("a path opened again after it opened is opened again", async () => {
  const { log, port } = steps((path) => Promise.resolve(path));
  const opener = new RepoOpener(port);

  await opener.open("/repo");
  await opener.open("/repo");

  assert.equal(log.filter((line) => line === "open /repo").length, 2);
});

void test("a failed open is reported once and the window returns to the welcome screen", async () => {
  const session = deferred<string>();
  const { log, port } = steps(() => session.promise);
  const opener = new RepoOpener(port);

  const first = opener.open("/nope");
  const second = opener.open("/nope");
  session.reject(new Error("not a repository"));
  await Promise.all([first, second]);

  assert.deepEqual(log, ["release", "open /nope", "failed /nope: not a repository", "mode welcome"]);
});
