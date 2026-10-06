// The shell's side of the transport (SPEC §4.1, I5): learn where the backend
// listens, authenticate, and correlate requests with responses. Rendering code
// builds on this; it contains no product logic.

import type { ClientMessage } from "./generated/ClientMessage";
import type { Ready } from "./generated/Ready";
import type { Request } from "./generated/Request";
import type { Response } from "./generated/Response";
import type { ServerMessage } from "./generated/ServerMessage";

// Must match PROTOCOL_VERSION in crates/vet-protocol.
const PROTOCOL_VERSION = 0;

/**
 * In Tauri the host spawned the backend and read its Ready line. In a plain
 * browser (development only, not a supported target) pass it in the URL hash:
 * `#port=1234&token=...`.
 */
async function backendAddress(): Promise<Ready> {
  if ("__TAURI_INTERNALS__" in window) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<Ready>("backend_connection");
  }
  const params = new URLSearchParams(location.hash.slice(1));
  const port = Number(params.get("port"));
  const token = params.get("token");
  if (!port || !token) throw new Error("open with #port=<port>&token=<token> outside Tauri");
  return { port, token };
}

export class Connection {
  private nextId = 0;
  private pending = new Map<number, { resolve: (r: Response) => void; reject: (e: Error) => void }>();

  private constructor(private socket: WebSocket) {
    socket.addEventListener("message", (event) => this.onMessage(JSON.parse(event.data) as ServerMessage));
    socket.addEventListener("close", () => this.failAll(new Error("connection closed")));
  }

  static async open(): Promise<Connection> {
    const { port, token } = await backendAddress();
    const socket = new WebSocket(`ws://127.0.0.1:${port}`);
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener("open", () => resolve(), { once: true });
      socket.addEventListener("error", () => reject(new Error("cannot reach backend")), { once: true });
    });
    const acknowledged = new Promise<void>((resolve, reject) => {
      socket.addEventListener(
        "message",
        (event) => {
          const message = JSON.parse(event.data) as ServerMessage;
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
    return new Connection(socket);
  }

  request(request: Request): Promise<Response> {
    const id = this.nextId++;
    const message: ClientMessage = { type: "request", id, request };
    return new Promise((resolve, reject) => {
      if (this.socket.readyState !== WebSocket.OPEN) return reject(new Error("connection closed"));
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify(message));
    });
  }

  private onMessage(message: ServerMessage) {
    if (message.type === "response") {
      this.pending.get(message.id)?.resolve(message.response);
      this.pending.delete(message.id);
    } else if (message.type === "error") {
      if (message.id === null) return console.error("backend:", message.message);
      this.pending.get(message.id)?.reject(new Error(message.message));
      this.pending.delete(message.id);
    }
  }

  private failAll(error: Error) {
    for (const { reject } of this.pending.values()) reject(error);
    this.pending.clear();
  }
}
