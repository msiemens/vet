import assert from "node:assert/strict";
import { test } from "node:test";
import type { Event as ServerEvent } from "../generated/Event";
import { type SessionEvent, projectEvents } from "./projectEvents.ts";

function setup() {
  const listeners = new Set<(event: ServerEvent) => void>();
  const heard: SessionEvent[] = [];
  const state: { open: number | undefined } = { open: 1 };
  const stop = projectEvents(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => state.open,
    (event) => heard.push(event),
  );
  const emit = (event: object) => {
    for (const listener of listeners) listener(event as ServerEvent);
  };
  return { state, heard, emit, stop };
}

const editsPending = (project: number) => ({ type: "editsPending", project, pending: true });

void test("an event of the open project is heard and one of another project is not", () => {
  const { heard, emit } = setup();
  emit(editsPending(1));
  emit(editsPending(2));
  assert.deepEqual(
    heard.map((event) => event.project),
    [1],
  );
});

void test("nothing is heard while no project is open", () => {
  const { state, heard, emit } = setup();
  state.open = undefined;
  emit(editsPending(1));
  assert.equal(heard.length, 0);
});

void test("after a reopen under a new id a late event of the old id is dropped", () => {
  const { state, heard, emit } = setup();
  state.open = 2;
  emit(editsPending(1));
  emit(editsPending(2));
  assert.deepEqual(
    heard.map((event) => event.project),
    [2],
  );
});

void test("the backend's own events never reach the listener", () => {
  const { heard, emit } = setup();
  emit({ type: "accountsChanged" });
  emit({ type: "preferenceChanged", project: 1 });
  assert.equal(heard.length, 0);
});

void test("a task is filtered by its own project", () => {
  const { heard, emit } = setup();
  emit({ type: "task", project: 2 });
  emit({ type: "task", project: 1 });
  assert.equal(heard.length, 1);
  assert.equal(heard[0]?.project, 1);
});

void test("unsubscribing stops the events", () => {
  const { heard, emit, stop } = setup();
  stop();
  emit(editsPending(1));
  assert.equal(heard.length, 0);
});
