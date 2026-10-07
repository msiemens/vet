import assert from "node:assert/strict";
import { test } from "node:test";
import type { StartupProgress, StartupStep } from "../../state/host.ts";
import { describe } from "./startup.ts";

const at = (step: StartupStep, distro: string | null = null, error: string | null = null): StartupProgress => ({
  step,
  distro,
  error,
});

void test("the bar counts the steps behind the current one and is not full while the last runs", () => {
  const steps: StartupStep[] = ["startingBackend", "waitingForReady", "loadingUi"];
  const views = steps.map((step) => describe(at(step)));
  assert.deepEqual(
    views.map((view) => view.done),
    [0, 1, 2],
  );
  assert.ok(views[2].done < views[2].total);
});

void test("the backend's step names where it runs, the interface step does not", () => {
  assert.equal(describe(at("startingBackend", "Ubuntu")).target, "WSL · Ubuntu");
  assert.equal(describe(at("waitingForReady")).target, "local");
  assert.equal(describe(at("loadingUi", "Ubuntu")).target, null);
});

void test("a failure shows the step it happened in with the message", () => {
  const view = describe(at("waitingForReady", "Ubuntu", "the backend did not print its Ready line in time"));
  assert.equal(view.error, "the backend did not print its Ready line in time");
  assert.equal(view.label, describe(at("waitingForReady")).label);
  assert.equal(view.done, 1);
});
