// The shell's side of the transport (SPEC §4.1, I5): authenticate, correlate requests with
// responses, and connect again when the socket drops. Rendering code builds on this; it contains
// no product logic.

import type { ClientMessage } from "../generated/ClientMessage";
import type { Event as ServerEvent } from "../generated/Event";
import type { LogLevel } from "../generated/LogLevel";
import type { Ready } from "../generated/Ready";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import type { ServerMessage } from "../generated/ServerMessage";
import { abortError } from "../lib/abort.ts";
import { LogBudget, reportFailedRequest } from "./errorLog.ts";
import { Pending } from "./pending.ts";

// Must match PROTOCOL_VERSION in crates/vet-protocol.
const PROTOCOL_VERSION = 2;

/** `WebSocket.OPEN`. */
const OPEN = 1;

/** The backend's `Ready` line and the repository to open; none shows the start screen. */
export type Address = Ready & { repo: string | null };

/** What the connection uses of a WebSocket. */
export interface Socket {
  readonly readyState: number;
  send(data: string): void;
  close(): void;
  addEventListener(
    type: "open" | "close" | "message",
    listener: (event: { data?: unknown }) => void,
    options?: { once: boolean },
  ): void;
}

/** Where the backend listens and how a socket to it is made. */
export interface Dialer {
  /** Asked again before each attempt: a backend the host started again listens elsewhere. */
  address(): Promise<Address>;
  socket(url: string): Socket;
}

/** Up; connecting again; connecting again for so long that the backend may need a restart. */
export type LinkState = "connected" | "reconnecting" | "stalled";

/** Milliseconds. */
export interface Timing {
  /** The waits before the attempts to connect again; the last one repeats. */
  retryDelays: readonly number[];
  /** How long an attempt waits for the backend to acknowledge the hello. */
  handshake: number;
  /** How long a request made while connecting again waits for the connection. */
  requestWait: number;
  /** How long connecting again takes before it counts as stalled. */
  stalledAfter: number;
}

export const TIMING: Timing = {
  retryDelays: [200, 500, 1000, 2000, 5000],
  handshake: 5000,
  requestWait: 10_000,
  stalledAfter: 30_000,
};

/** Opens a socket to the backend and authenticates on it; rejects if the backend cannot be reached or refuses. */
async function dial(dialer: Dialer, timeout: number): Promise<{ socket: Socket; repo: string | null }> {
  const { port, token, repo } = await dialer.address();
  const socket = dialer.socket(`ws://127.0.0.1:${port}`);
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    await new Promise<void>((resolve, reject) => {
      let opened = false;
      timer = setTimeout(() => reject(new Error("the backend did not answer")), timeout);
      socket.addEventListener(
        "open",
        () => {
          opened = true;
          const hello: ClientMessage = { type: "hello", token, protocolVersion: PROTOCOL_VERSION };
          socket.send(JSON.stringify(hello));
        },
        { once: true },
      );
      socket.addEventListener(
        "message",
        (event) => {
          const message = JSON.parse(event.data as string) as ServerMessage;
          if (message.type === "helloAck") resolve();
          else reject(new Error("unexpected first message from backend"));
        },
        { once: true },
      );
      socket.addEventListener(
        "close",
        () => reject(new Error(opened ? "backend rejected the handshake" : "cannot reach backend")),
        { once: true },
      );
    });
  } catch (error) {
    socket.close();
    throw error;
  } finally {
    clearTimeout(timer);
  }
  return { socket, repo };
}

export class Connection {
  /** How the connection is; `onState` hears it change. */
  state: LinkState = "connected";
  /** The repository to open first, if one was named. */
  readonly repo: string | null;
  private socket: Socket;
  private dialer: Dialer;
  private timing: Timing;
  private nextId = 0;
  private pending = new Pending<Response>();
  private listeners = new Set<(event: ServerEvent) => void>();
  private stateListeners = new Set<(state: LinkState) => void>();
  private reconnectListeners = new Set<() => void>();
  /** The requests made while connecting again, waiting for the connection. */
  private waiting = new Set<() => void>();
  private logBudget = new LogBudget();
  /** How many attempts to connect again failed in a row. */
  private attempts = 0;
  private dialing = false;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private stalledTimer?: ReturnType<typeof setTimeout>;

  private constructor(dialer: Dialer, timing: Timing, socket: Socket, repo: string | null) {
    this.dialer = dialer;
    this.timing = timing;
    this.repo = repo;
    this.socket = socket;
    this.attach(socket);
  }

  static async open(dialer: Dialer, timing: Timing = TIMING): Promise<Connection> {
    const { socket, repo } = await dial(dialer, timing.handshake);
    return new Connection(dialer, timing, socket, repo);
  }

  private attach(socket: Socket) {
    this.socket = socket;
    socket.addEventListener("message", (event) => this.onMessage(JSON.parse(event.data as string) as ServerMessage));
    socket.addEventListener("close", () => this.dropped());
  }

  /**
   * The socket closed. The requests that were on it fail: the backend may have carried them out, so
   * they are not sent again. Then connecting again starts.
   */
  private dropped() {
    this.pending.rejectAll(new Error("connection closed"));
    this.setState("reconnecting");
    this.stalledTimer = setTimeout(() => this.setState("stalled"), this.timing.stalledAfter);
    this.attempts = 0;
    this.scheduleRetry();
  }

  private scheduleRetry() {
    const delays = this.timing.retryDelays;
    const delay = delays[Math.min(this.attempts, delays.length - 1)];
    this.attempts++;
    this.retryTimer = setTimeout(() => void this.redial(), delay);
  }

  /** Tries to connect again at once and, if that fails, with the shortest waits again: the backend was restarted. */
  retryNow() {
    if (this.state === "connected" || this.dialing) return;
    clearTimeout(this.retryTimer);
    this.attempts = 0;
    void this.redial();
  }

  private async redial() {
    this.dialing = true;
    let socket: Socket;
    try {
      ({ socket } = await dial(this.dialer, this.timing.handshake));
    } catch {
      this.scheduleRetry();
      return;
    } finally {
      this.dialing = false;
    }
    this.attach(socket);
    clearTimeout(this.stalledTimer);
    this.setState("connected");
    for (const resume of [...this.waiting]) resume();
    for (const listener of this.reconnectListeners) listener();
  }

  private setState(state: LinkState) {
    if (state === this.state) return;
    this.state = state;
    for (const listener of this.stateListeners) listener(state);
  }

  /** Calls `listener` whenever the connection drops, stalls or is up again; returns the unsubscribe function. */
  onState(listener: (state: LinkState) => void): () => void {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  /**
   * Calls `listener` when the connection is up again. The backend knows nothing of the window then: what
   * was opened or subscribed to on the dropped socket is to be asked for again.
   */
  onReconnect(listener: () => void): () => void {
    this.reconnectListeners.add(listener);
    return () => this.reconnectListeners.delete(listener);
  }

  /**
   * Aborting `signal` gives the request up: the backend is told to cancel it and the promise rejects with an
   * abort. A request made while the connection is down waits for it a while.
   */
  request(request: Request, signal?: AbortSignal): Promise<Response> {
    const answer = this.ask(request, signal);
    void answer.catch((error: unknown) =>
      reportFailedRequest((level, message, context) => this.log(level, message, context), request.type, error),
    );
    return answer;
  }

  private ask(request: Request, signal?: AbortSignal): Promise<Response> {
    if (signal?.aborted) return Promise.reject(abortError());
    if (this.socket.readyState !== OPEN) return this.connected(signal).then(() => this.ask(request, signal));
    const id = this.nextId++;
    const answer = this.pending.wait(id, signal, (cancelled) => {
      if (this.socket.readyState === OPEN) this.send({ type: "cancel", id: cancelled });
    });
    this.send({ type: "request", id, request });
    return answer;
  }

  /** Resolves once the connection is up again; rejects after `requestWait`, or with an abort when `signal` fires. */
  private connected(signal?: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      const settle = () => {
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
        this.waiting.delete(resume);
      };
      const resume = () => {
        settle();
        resolve();
      };
      const onAbort = () => {
        settle();
        reject(abortError());
      };
      const timer = setTimeout(() => {
        settle();
        reject(new Error("no connection to the backend"));
      }, this.timing.requestWait);
      signal?.addEventListener("abort", onAbort, { once: true });
      this.waiting.add(resume);
    });
  }

  /** Adds an entry to the backend's log; a few per interval are sent, the rest and those while the connection is down dropped. */
  log(level: LogLevel, message: string, context: string) {
    if (this.socket.readyState !== OPEN || !this.logBudget.admit(performance.now())) return;
    this.send({ type: "log", level, message, context });
  }

  private send(message: ClientMessage) {
    this.socket.send(JSON.stringify(message));
  }

  /** Events of the projects opened on this connection; returns the unsubscribe function. */
  onEvent(listener: (event: ServerEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private onMessage(message: ServerMessage) {
    if (message.type === "event") {
      for (const listener of this.listeners) listener(message.event);
    } else if (message.type === "response") {
      this.pending.resolve(message.id, message.response);
    } else if (message.type === "error") {
      if (message.id === null) {
        console.error("backend:", message.message);
        return;
      }
      this.pending.reject(message.id, new Error(message.message));
    } else if (message.type === "cancelled") {
      this.pending.reject(message.id, abortError());
    }
  }
}
