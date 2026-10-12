// Runs one scripted session in this process (the stores are singletons, so each session gets a process
// of its own) and prints what its steps found as a last line `RESULT <json>`. Used by `run.ts`.

import { runSession } from "./harness.ts";
import { sessions } from "./sessions.ts";

const name = process.argv[2];
if (!Object.hasOwn(sessions, name)) throw new Error(`no session named ${name}`);

const steps = await runSession(sessions[name]);
process.stdout.write(`RESULT ${JSON.stringify(steps)}\n`);
// The stores keep timers and sockets open.
process.exit(0);
