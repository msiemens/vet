// The shell's side of the transport (SPEC §4.1, I5): learn where the backend
// listens, authenticate, and correlate requests with responses. Rendering code
// builds on this; it contains no product logic.

import type { ClientMessage } from "../generated/ClientMessage";
import type { Event as ServerEvent } from "../generated/Event";
import type { Ready } from "../generated/Ready";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import type { ServerMessage } from "../generated/ServerMessage";
import { abortError } from "../lib/abort";
import { hashParams, inTauri, invoke } from "./host";
import { Pending } from "./pending";

// Must match PROTOCOL_VERSION in crates/vet-protocol.
const PROTOCOL_VERSION = 2;

/** The backend's `Ready` line and the repository to open; none shows the start screen. */
type Address = Ready & { repo: string | null };

/**
 * In Tauri the host spawned the backend, read its Ready line and took the
 * repository from its command line. In a plain browser (development only, not a
 * supported target) pass both in the URL hash: `#port=1234&token=...&repo=/path`
 * (without `repo`, the start screen asks for one).
 */
async function backendAddress(): Promise<Address> {
  if (inTauri) return invoke<Address>("backend_connection");
  const params = hashParams();
  const port = Number(params.get("port"));
  const token = params.get("token");
  if (!port || !token) throw new Error("open with #port=<port>&token=<token> outside Tauri");
  return { port, token, repo: params.get("repo") };
}

export class Connection {
  private nextId = 0;
  private pending = new Pending<Response>();
  private listeners = new Set<(event: ServerEvent) => void>();

  private constructor(
    private socket: WebSocket,
    /** The repository to open first, if one was named. */
    readonly repo: string | null,
  ) {
    socket.addEventListener("message", (event) => this.onMessage(JSON.parse(event.data as string) as ServerMessage));
    socket.addEventListener("close", () => this.failAll(new Error("connection closed")));
  }

  static async open(): Promise<Connection> {
    const { port, token, repo } = await backendAddress();
    const socket = new WebSocket(`ws://127.0.0.1:${port}`);
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener("open", () => resolve(), { once: true });
      socket.addEventListener("error", () => reject(new Error("cannot reach backend")), { once: true });
    });
    const acknowledged = new Promise<void>((resolve, reject) => {
      socket.addEventListener(
        "message",
        (event) => {
          const message = JSON.parse(event.data as string) as ServerMessage;
          if (message.type === "helloAck") resolve();
          else reject(new Error("unexpected first message from backend"));
        },
        { once: true },
      );
      socket.addEventListener("close", () => reject(new Error("backend rejected the handshake")), { once: true });
    });
    const hello: ClientMessage = { type: "hello", token, protocolVersion: PROTOCOL_VERSION };
    socket.send(JSON.stringify(hello));
    await acknowledged;
    return new Connection(socket, repo);
  }

  /** Aborting `signal` gives the request up: the backend is told to cancel it and the promise rejects with an abort. */
  request(request: Request, signal?: AbortSignal): Promise<Response> {
    if (signal?.aborted) return Promise.reject(abortError());
    if (this.socket.readyState !== WebSocket.OPEN) return Promise.reject(new Error("connection closed"));
    const id = this.nextId++;
    const answer = this.pending.wait(id, signal, (cancelled) => {
      if (this.socket.readyState === WebSocket.OPEN) this.send({ type: "cancel", id: cancelled });
    });
    this.send({ type: "request", id, request });
    return answer;
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

  private failAll(error: Error) {
    this.pending.rejectAll(error);
  }
}
