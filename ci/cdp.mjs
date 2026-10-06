// Raw CDP client for the shared browser (ci/browser.sh), library and CLI. No dependencies.
//
//   import { withTab, withBenchLock } from "./cdp.mjs";
//   await withTab(async (tab) => { await tab.navigate(url); await tab.screenshot("a.png"); });
//
//   node ci/cdp.mjs screenshot <url> <out.png> [--wait ms] [--theme light|dark]
//   node ci/cdp.mjs watchdog        (started by browser.sh)
import { execFileSync, spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function ensureBrowser() {
    const out = execFileSync(join(here, "browser.sh"), { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
    return Number(out.trim().split("\n").pop());
}

async function connect(port) {
    const { webSocketDebuggerUrl } = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
    const ws = new WebSocket(webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
        ws.onopen = resolve;
        ws.onerror = () => reject(new Error("CDP connection failed"));
    });
    let nextId = 1;
    const pending = new Map();
    const listeners = [];
    ws.onmessage = ({ data }) => {
        const msg = JSON.parse(data);
        if (msg.id) {
            const p = pending.get(msg.id);
            pending.delete(msg.id);
            if (msg.error) p?.reject(new Error(`${p.method}: ${msg.error.message}`));
            else p?.resolve(msg.result);
        } else {
            for (const l of listeners) l(msg);
        }
    };
    ws.onclose = () => {
        for (const p of pending.values()) p.reject(new Error("CDP connection closed"));
    };
    return {
        send: (method, params = {}, sessionId) =>
            new Promise((resolve, reject) => {
                const id = nextId++;
                pending.set(id, { resolve, reject, method });
                ws.send(JSON.stringify({ id, method, params, sessionId }));
            }),
        on: (l) => listeners.push(l),
        close: () => ws.close(),
    };
}

/** Runs fn(tab) in a fresh, isolated browser context and disposes it afterwards. */
export async function withTab(fn, { width = 1440, height = 900 } = {}) {
    const conn = await connect(ensureBrowser());
    let contextId;
    const dispose = async () => {
        if (!contextId) return;
        const id = contextId;
        contextId = undefined;
        await conn.send("Target.disposeBrowserContext", { browserContextId: id }).catch(() => {});
    };
    const onSignal = (code) => () => dispose().finally(() => process.exit(code));
    const sigint = onSignal(130);
    const sigterm = onSignal(143);
    process.once("SIGINT", sigint);
    process.once("SIGTERM", sigterm);
    try {
        ({ browserContextId: contextId } = await conn.send("Target.createBrowserContext"));
        const { targetId } = await conn.send("Target.createTarget", { url: "about:blank", browserContextId: contextId });
        const { sessionId } = await conn.send("Target.attachToTarget", { targetId, flatten: true });
        const send = (method, params) => conn.send(method, params, sessionId);

        const console_ = [];
        const exceptions = [];
        const waiters = [];
        conn.on((msg) => {
            if (msg.sessionId !== sessionId) return;
            const p = msg.params;
            if (msg.method === "Runtime.consoleAPICalled") {
                console_.push({ type: p.type, text: p.args.map((a) => a.value ?? a.description ?? "").join(" ") });
            } else if (msg.method === "Runtime.exceptionThrown") {
                exceptions.push(p.exceptionDetails.exception?.description ?? p.exceptionDetails.text);
            }
            for (const w of waiters.filter((w) => w.method === msg.method)) w.resolve(p);
        });
        const once = (method) => new Promise((resolve) => waiters.push({ method, resolve }));

        await send("Page.enable");
        await send("Runtime.enable");
        await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });

        const tab = {
            send,
            console: console_,
            exceptions,
            async evaluate(expression) {
                const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
                if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
                return r.result.value;
            },
            async navigate(url) {
                const loaded = once("Page.loadEventFired");
                const { errorText } = await send("Page.navigate", { url });
                if (errorText) throw new Error(`navigate ${url}: ${errorText}`);
                await loaded;
            },
            async screenshot(path, clip) {
                const { data } = await send("Page.captureScreenshot", {
                    format: "png",
                    ...(clip && { clip: { scale: 1, ...clip } }),
                });
                writeFileSync(path, Buffer.from(data, "base64"));
            },
            /** type: mouseMoved | mousePressed | mouseReleased | mouseWheel */
            mouse: (type, x, y, extra = {}) => send("Input.dispatchMouseEvent", { type, x, y, ...extra }),
        };
        return await fn(tab);
    } finally {
        process.off("SIGINT", sigint);
        process.off("SIGTERM", sigterm);
        await dispose();
        conn.close();
    }
}

/** Runs fn while holding the exclusive benchmark lock; parallel benchmarks skew each other. */
export async function withBenchLock(fn) {
    // flock runs `cat`, which holds the lock until stdin closes (also when this process dies).
    const holder = spawn("flock", ["-x", "/tmp/vet-bench.lock", "sh", "-c", "echo locked; cat"], {
        stdio: ["pipe", "pipe", "inherit"],
    });
    await new Promise((resolve, reject) => {
        holder.stdout.once("data", resolve);
        holder.once("error", reject);
    });
    try {
        return await fn();
    } finally {
        holder.stdin.end();
    }
}

/** Closes the browser once no real page (not about:blank or an internal one) existed for `idle` seconds. */
async function watchdog() {
    const port = Number(process.env.VET_BROWSER_PORT ?? 9222);
    const idleMs = Number(process.env.VET_BROWSER_IDLE ?? 600) * 1000;
    let lastBusy = Date.now();
    let failures = 0;
    for (;;) {
        await sleep(2000);
        try {
            const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
            failures = 0;
            // Edge opens an internal edge:// dialog page by itself.
            if (targets.some((t) => t.type === "page" && !/^(about:blank|edge:|chrome:)/.test(t.url))) lastBusy = Date.now();
        } catch {
            if (++failures >= 3) return;
            continue;
        }
        if (Date.now() - lastBusy >= idleMs) {
            const conn = await connect(port);
            await conn.send("Browser.close").catch(() => {});
            return;
        }
    }
}

async function screenshotCli(args) {
    const flags = {};
    const positional = [];
    for (let i = 0; i < args.length; i++) {
        if (args[i].startsWith("--")) flags[args[i].slice(2)] = args[++i];
        else positional.push(args[i]);
    }
    const [url, out] = positional;
    if (!url || !out) throw new Error("usage: cdp.mjs screenshot <url> <out.png> [--wait ms] [--theme light|dark]");
    await withTab(async (tab) => {
        if (flags.theme) {
            await tab.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: flags.theme }] });
        }
        await tab.navigate(url);
        await sleep(Number(flags.wait ?? 500));
        await tab.screenshot(out);
        for (const e of tab.exceptions) console.error("exception:", e);
    });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
    const [cmd, ...args] = process.argv.slice(2);
    const run = { screenshot: screenshotCli, watchdog };
    if (!run[cmd]) {
        console.error("usage: cdp.mjs screenshot <url> <out.png> | watchdog");
        process.exit(2);
    }
    run[cmd](args).then(
        () => process.exit(0),
        (e) => {
            console.error(e.message);
            process.exit(1);
        },
    );
}
