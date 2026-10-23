// Sends trusted input to the page under test over the DevTools protocol, when the
// page asks through its title, and answers through the title:
// - a key press: `RINCH-REAL-KEY:<n>:<key>:<shift 0|1>` → `RINCH-REAL-KEY-SENT:<n>`
//   (`editor_node_view_real_tab.rs`);
// - a touch: `RINCH-REAL-TOUCH:<n>:<x>,<y>[;<x>,<y>…]` → `RINCH-REAL-TOUCH-SENT:<n>`,
//   one finger down at the first point (viewport CSS px), moved through the rest,
//   lifted at the last (`real_touch_1582_1588.rs`);
// - a mouse click: `RINCH-REAL-CLICK:<n>:<x>,<y>` → `RINCH-REAL-CLICK-SENT:<n>`, the
//   primary button pressed and released at that point (viewport CSS px), which
//   gives the page a user activation (`clipboard_copy_then.rs`);
// - a clipboard grant: `RINCH-GRANT-CLIPBOARD:<n>` → `RINCH-GRANT-CLIPBOARD-SENT:<n>`,
//   `Browser.grantPermissions` of clipboard read and write to the page's origin, so
//   the page can read back what it copied (`clipboard_copy_then.rs`).
// Used by `run_real_keys.sh`. Usage: node real_keys.mjs <debugging port>
//
// Each test binary runs in a Chrome of its own on the same port, so the pages this
// polls come and go. A CDP call on a page that has gone never gets an answer, and
// before #1839 the helper awaited one for ever: Chrome closed between two test
// binaries while the helper was reading a page's title, the await never settled,
// Node exited ("unsettled top-level await"), and the next binary's test waited for
// a click nobody would send. So every call now fails when its socket closes or
// after `CALL_TIMEOUT_MS`, a session that fails is dropped (and attached again if
// the page is still listed), and the helper runs until `run_real_keys.sh` kills it.
// Everything it does is logged to stderr with the time, for the runner to show
// when a test fails.
const port = process.argv[2];
const CALL_TIMEOUT_MS = 5_000;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const t0 = Date.now();
const log = (...a) => console.error(`[real_keys ${((Date.now() - t0) / 1000).toFixed(2)}s]`, ...a);
const done = new Set();

async function session(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((ok, err) => {
    ws.onopen = ok;
    ws.onerror = () => err(new Error("websocket error while connecting"));
    ws.onclose = () => err(new Error("websocket closed while connecting"));
  });
  let id = 0;
  const pending = new Map();
  const s = { ws, closed: false };
  const failAll = (why) => {
    s.closed = true;
    for (const { fail } of pending.values()) fail(new Error(why));
    pending.clear();
  };
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    const p = pending.get(msg.id);
    if (p) { pending.delete(msg.id); p.ok(msg); }
  };
  ws.onclose = () => failAll("websocket closed");
  ws.onerror = () => failAll("websocket error");
  s.send = (method, params = {}) => new Promise((ok, fail) => {
    if (s.closed || ws.readyState !== WebSocket.OPEN) { fail(new Error("websocket not open")); return; }
    const n = ++id;
    const timer = setTimeout(() => {
      pending.delete(n);
      fail(new Error(`${method} timed out after ${CALL_TIMEOUT_MS}ms`));
    }, CALL_TIMEOUT_MS);
    pending.set(n, {
      ok: (msg) => { clearTimeout(timer); ok(msg); },
      fail: (e) => { clearTimeout(timer); fail(e); },
    });
    ws.send(JSON.stringify({ id: n, method, params }));
  });
  return s;
}

async function key(s, key, shift) {
  const modifiers = shift ? 8 : 0;
  const base = { key, code: key, windowsVirtualKeyCode: key === "Tab" ? 9 : 0, modifiers };
  if (shift) await s.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16, modifiers });
  await s.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  await s.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  if (shift) await s.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16, modifiers: 0 });
}

async function touch(s, points) {
  const [first, ...rest] = points;
  // A touch-enabled page, as on a phone: without it Chrome does not treat the
  // finger as one that can pan.
  await s.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
  await s.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [first] });
  for (const p of rest) {
    await sleep(16);
    await s.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [p] });
  }
  await sleep(16);
  await s.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  // What follows a tap (compatibility mouse events, the click) comes after the
  // touch ends; leave the emulation on until it has.
  await sleep(300);
  await s.send("Emulation.setTouchEmulationEnabled", { enabled: false });
}

async function click(s, x, y) {
  const base = { x, y, button: "left", clickCount: 1 };
  await s.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await s.send("Input.dispatchMouseEvent", { type: "mousePressed", buttons: 1, ...base });
  await s.send("Input.dispatchMouseEvent", { type: "mouseReleased", buttons: 0, ...base });
}

// Answers what one page's title asks for, if anything not answered yet.
async function serve(p, s) {
  const r = await s.send("Runtime.evaluate", { expression: "document.title", returnByValue: true });
  const title = r.result?.result?.value ?? "";
  if (lastTitle.get(p.id) !== title) {
    lastTitle.set(p.id, title);
    log(`page ${p.id} title ${JSON.stringify(title)}`);
  }
  const m = /^RINCH-REAL-KEY:(\d+):([^:]+):([01])$/.exec(title);
  if (m && !done.has(`${p.id}:${m[1]}`)) {
    done.add(`${p.id}:${m[1]}`);
    await key(s, m[2], m[3] === "1");
    await s.send("Runtime.evaluate", { expression: `document.title = "RINCH-REAL-KEY-SENT:${m[1]}"` });
    log(`sent #${m[1]} ${m[2]} shift=${m[3]}`);
  }
  const t = /^RINCH-REAL-TOUCH:(\d+):([-0-9.,;]+)$/.exec(title);
  if (t && !done.has(`${p.id}:touch:${t[1]}`)) {
    done.add(`${p.id}:touch:${t[1]}`);
    const points = t[2].split(";").map((xy) => {
      const [x, y] = xy.split(",").map(Number);
      return { x, y };
    });
    await touch(s, points);
    await s.send("Runtime.evaluate", { expression: `document.title = "RINCH-REAL-TOUCH-SENT:${t[1]}"` });
    log(`touched #${t[1]} ${t[2]}`);
  }
  const c = /^RINCH-REAL-CLICK:(\d+):([-0-9.]+),([-0-9.]+)$/.exec(title);
  if (c && !done.has(`${p.id}:click:${c[1]}`)) {
    done.add(`${p.id}:click:${c[1]}`);
    await click(s, Number(c[2]), Number(c[3]));
    await s.send("Runtime.evaluate", { expression: `document.title = "RINCH-REAL-CLICK-SENT:${c[1]}"` });
    log(`clicked #${c[1]} ${c[2]},${c[3]}`);
  }
  const g = /^RINCH-GRANT-CLIPBOARD:(\d+)$/.exec(title);
  if (g && !done.has(`${p.id}:grant:${g[1]}`)) {
    done.add(`${p.id}:grant:${g[1]}`);
    const o = await s.send("Runtime.evaluate", { expression: "location.origin", returnByValue: true });
    const origin = o.result?.result?.value;
    const r = await s.send("Browser.grantPermissions", { origin, permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"] });
    const outcome = r.error ? `ERR ${r.error.message}` : "OK";
    await s.send("Runtime.evaluate", { expression: `document.title = "RINCH-GRANT-CLIPBOARD-SENT:${g[1]}:${outcome}"` });
    log(`granted clipboard #${g[1]} to ${origin}: ${outcome}`);
  }
}

const sessions = new Map();
const lastTitle = new Map();
let reachable = null;
log(`polling 127.0.0.1:${port}`);
for (;;) {
  let pages = [];
  try {
    pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    if (reachable !== true) { reachable = true; log("DevTools port answers"); }
  } catch {
    if (reachable !== false) { reachable = false; log("DevTools port does not answer (no Chrome yet, or between two)"); }
    await sleep(100);
    continue;
  }
  pages = pages.filter((p) => p.type === "page");
  const listed = new Set(pages.map((p) => p.id));
  for (const [id, s] of sessions) {
    if (!listed.has(id) || s.closed) {
      sessions.delete(id);
      s.ws.close();
      log(`page ${id} gone`);
    }
  }
  for (const p of pages) {
    if (!sessions.has(p.id)) {
      try {
        sessions.set(p.id, await session(p.webSocketDebuggerUrl));
        log(`page ${p.id} attached (${p.url})`);
      } catch (e) {
        log(`page ${p.id} attach failed: ${e.message}`);
        continue;
      }
    }
    try {
      await serve(p, sessions.get(p.id));
    } catch (e) {
      // A request that failed half way stays done: some of its input may have
      // reached the page, and sending it again could press twice. A call fails
      // on a page that is going away, and the test's own wait says if it was not.
      log(`page ${p.id}: ${e.message}; dropping the session`);
      const s = sessions.get(p.id);
      sessions.delete(p.id);
      s.ws.close();
    }
  }
  await sleep(50);
}
