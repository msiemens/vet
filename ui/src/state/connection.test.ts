import assert from "node:assert/strict";
import { afterEach, beforeEach, mock, test } from "node:test";
import type { ClientMessage } from "../generated/ClientMessage";
import type { Event as ServerEvent } from "../generated/Event";
import type { ServerMessage } from "../generated/ServerMessage";
import { type Address, Connection, type Dialer, type LinkState, type Socket } from "./connection.ts";

type Listener = (event: { data?: unknown }) => void;

class FakeSocket implements Socket {
  readyState = 0;
  sent: ClientMessage[] = [];
  private handlers = new Map<string, { listener: Listener; once: boolean }[]>();
  readonly url: string;
  private backend: FakeBackend;

  constructor(url: string, backend: FakeBackend) {
    this.url = url;
    this.backend = backend;
  }

  addEventListener(type: string, listener: Listener, options?: { once: boolean }) {
    const list = this.handlers.get(type) ?? [];
    list.push({ listener, once: options?.once ?? false });
    this.handlers.set(type, list);
  }

  private emit(type: string, event: { data?: unknown } = {}) {
    const list = this.handlers.get(type) ?? [];
    this.handlers.set(
      type,
      list.filter((h) => !h.once),
    );
    for (const { listener } of list) listener(event);
  }

  send(data: string) {
    const message = JSON.parse(data) as ClientMessage;
    this.sent.push(message);
    this.backend.received(this, message);
  }

  close() {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.emit("close");
  }

  open() {
    this.readyState = 1;
    this.emit("open");
  }

  push(message: ServerMessage) {
    this.emit("message", { data: JSON.stringify(message) });
  }

  requests() {
    return this.sent.filter((m) => m.type === "request");
  }
}

/** A backend that accepts sockets while `up` and acknowledges the hello that carries its token. */
class FakeBackend implements Dialer {
  up = true;
  address_: Address = { port: 4000, token: "secret", repo: "/repo" };
  sockets: FakeSocket[] = [];
  /** When each socket was asked for, in fake milliseconds. */
  dialedAt: number[] = [];

  address() {
    return Promise.resolve(this.address_);
  }

  socket(url: string): Socket {
    const socket = new FakeSocket(url, this);
    this.sockets.push(socket);
    this.dialedAt.push(Date.now());
    const up = this.up;
    queueMicrotask(() => (up ? socket.open() : socket.close()));
    return socket;
  }

  received(socket: FakeSocket, message: ClientMessage) {
    if (message.type !== "hello") return;
    if (message.token === this.address_.token) socket.push({ type: "helloAck", protocolVersion: 2 });
    else socket.close();
  }

  get last(): FakeSocket {
    const socket = this.sockets.at(-1);
    assert.ok(socket);
    return socket;
  }
}

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

/** Moves the fake clock on in small steps, letting what each timer starts run. */
async function advance(ms: number) {
  for (let passed = 0; passed < ms; passed += 10) {
    mock.timers.tick(Math.min(10, ms - passed));
    await settle();
  }
}

async function connect(backend: FakeBackend): Promise<Connection> {
  const opening = Connection.open(backend);
  await settle();
  return opening;
}

function watch(conn: Connection) {
  const states: LinkState[] = [];
  let reconnects = 0;
  conn.onState((state) => states.push(state));
  conn.onReconnect(() => reconnects++);
  return { states, reconnects: () => reconnects };
}

beforeEach(() => mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 }));
afterEach(() => mock.timers.reset());

void test("a dropped socket is replaced by one to the same address, which authenticates and hears the events", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);
  const seen = watch(conn);
  const events: ServerEvent[] = [];
  conn.onEvent((event) => events.push(event));

  backend.sockets[0].close();
  assert.equal(conn.state, "reconnecting");
  await advance(200);

  assert.equal(backend.sockets.length, 2);
  assert.equal(backend.last.url, "ws://127.0.0.1:4000");
  assert.deepEqual(backend.last.sent[0], { type: "hello", token: "secret", protocolVersion: 2 });
  assert.deepEqual(seen.states, ["reconnecting", "connected"]);
  assert.equal(seen.reconnects(), 1);
  const event: ServerEvent = { type: "serversChanged", project: 1 };
  backend.last.push({ type: "event", event });
  assert.deepEqual(events, [event]);
});

void test("a request on the socket that dropped fails and is not sent again", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);

  const answer = conn.request({ type: "recentProjects" });
  backend.sockets[0].close();
  await assert.rejects(answer, /connection closed/);
  await advance(200);

  assert.equal(conn.state, "connected");
  assert.deepEqual(backend.last.requests(), []);
});

void test("a request made while connecting again waits and goes out on the new socket", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);
  backend.up = false;
  backend.sockets[0].close();

  const answer = conn.request({ type: "recentProjects" });
  await advance(700);
  backend.up = true;
  await advance(1000);

  const sent = backend.last.requests().at(0);
  assert.ok(sent, "the request went out");
  assert.equal(sent.request.type, "recentProjects");
  backend.last.push({ type: "response", id: sent.id, response: { type: "recentProjects", projects: [] } });
  assert.deepEqual(await answer, { type: "recentProjects", projects: [] });
});

void test("a request made while connecting again gives up after ten seconds, or when it is aborted", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);
  backend.up = false;
  backend.sockets[0].close();

  const waiting = conn.request({ type: "recentProjects" });
  const controller = new AbortController();
  const aborted = conn.request({ type: "settings" }, controller.signal);
  controller.abort();
  await assert.rejects(aborted, { name: "AbortError" });
  await advance(9990);
  let failed: unknown;
  waiting.catch((error: unknown) => (failed = error));
  await settle();
  assert.equal(failed, undefined);
  await advance(10);
  await assert.rejects(waiting, /no connection to the backend/);
});

void test("attempts wait 0.2, 0.5, 1 and 2 seconds, then 5 seconds each; after 30 seconds it is stalled", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);
  const seen = watch(conn);
  backend.up = false;
  backend.sockets[0].close();

  await advance(29_990);
  assert.deepEqual(seen.states, ["reconnecting"]);
  await advance(10);
  assert.deepEqual(seen.states, ["reconnecting", "stalled"]);
  assert.deepEqual(backend.dialedAt.slice(1), [200, 700, 1700, 3700, 8700, 13700, 18700, 23700, 28700]);

  backend.up = true;
  await advance(5000);
  assert.deepEqual(seen.states, ["reconnecting", "stalled", "connected"]);
});

void test("retrying at once after a restart reaches the new address and starts with short waits again", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);
  backend.up = false;
  backend.sockets[0].close();
  await advance(31_000);
  const before = backend.sockets.length;

  backend.address_ = { port: 5000, token: "fresh", repo: "/repo" };
  conn.retryNow();
  await advance(150);
  assert.equal(backend.sockets.length, before + 1);
  await advance(200);
  assert.equal(backend.sockets.length, before + 2, "the next attempt follows after 0.2 s");

  backend.up = true;
  await advance(500);
  assert.equal(conn.state, "connected");
  assert.equal(backend.last.url, "ws://127.0.0.1:5000");
});

void test("a backend that refuses the token counts as a failed attempt", async () => {
  const backend = new FakeBackend();
  const conn = await connect(backend);
  backend.received = (socket, message) => {
    if (message.type === "hello") socket.close();
  };
  backend.sockets[0].close();

  await advance(200);
  assert.equal(conn.state, "reconnecting");
  assert.equal(backend.sockets.length, 2);
  await advance(500);
  assert.equal(backend.sockets.length, 3);
});
