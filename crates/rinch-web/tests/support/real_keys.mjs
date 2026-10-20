// Sends trusted input to the page under test over the DevTools protocol, when the
// page asks through its title, and answers through the title:
// - a key press: `RINCH-REAL-KEY:<n>:<key>:<shift 0|1>` → `RINCH-REAL-KEY-SENT:<n>`
//   (`editor_node_view_real_tab.rs`);
// - a touch: `RINCH-REAL-TOUCH:<n>:<x>,<y>[;<x>,<y>…]` → `RINCH-REAL-TOUCH-SENT:<n>`,
//   one finger down at the first point (viewport CSS px), moved through the rest,
//   lifted at the last (`real_touch_1582_1588.rs`).
// Used by `run_real_keys.sh`. Usage: node real_keys.mjs <debugging port>
const port = process.argv[2];
const deadline = Date.now() + 300_000;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const done = new Set();

async function session(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((ok, err) => { ws.onopen = ok; ws.onerror = err; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  };
  const send = (method, params = {}) => new Promise((ok) => {
    const n = ++id;
    pending.set(n, ok);
    ws.send(JSON.stringify({ id: n, method, params }));
  });
  return { ws, send };
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

const sessions = new Map();
while (Date.now() < deadline) {
  let pages = [];
  try { pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); } catch { await sleep(200); continue; }
  for (const p of pages.filter((p) => p.type === "page")) {
    if (!sessions.has(p.id)) {
      try { sessions.set(p.id, await session(p.webSocketDebuggerUrl)); } catch { continue; }
    }
    const s = sessions.get(p.id);
    const r = await s.send("Runtime.evaluate", { expression: "document.title", returnByValue: true });
    const title = r.result?.result?.value ?? "";
    const m = /^RINCH-REAL-KEY:(\d+):([^:]+):([01])$/.exec(title);
    if (m && !done.has(`${p.id}:${m[1]}`)) {
      done.add(`${p.id}:${m[1]}`);
      await key(s, m[2], m[3] === "1");
      await s.send("Runtime.evaluate", { expression: `document.title = "RINCH-REAL-KEY-SENT:${m[1]}"` });
      console.log(`real_keys: sent #${m[1]} ${m[2]} shift=${m[3]}`);
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
      console.log(`real_keys: touched #${t[1]} ${t[2]}`);
    }
  }
  await sleep(50);
}
for (const s of sessions.values()) s.ws.close();
