// The emulator and a passkey, end to end in Chrome: node web/e2e.mjs [screenshot.png]
// E2E_URL=https://diykey.app/ runs it against the deployed site instead of this checkout.
// Serves the repository on localhost, starts headless Chrome with a virtual WebAuthn authenticator
// (the DevTools protocol's), and drives the page with real key presses: make a passkey, send a
// transaction, page to the hash, sign, and check the signature verified; then a wrong claim, which
// must not sign. Then the key ceremony from a pretend Cardputer over Web Serial, and last, the key
// ceremony's page at a phone's width. Needs `web/build.sh` first, and Chrome (CHROME=/path/to/chrome if not found).
import { spawn } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { extname, join, normalize } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const CHROME = process.env.CHROME || [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium",
  "/usr/bin/chromium-browser"].find(existsSync);
if (!CHROME) { console.error("no Chrome: set CHROME=/path/to/chrome"); process.exit(2); }
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css",
  ".json": "application/json", ".png": "image/png", ".wasm": "application/wasm", ".bin": "application/octet-stream" };

const server = createServer((req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(/^(\.\.[/\\])+/, "");
  let file = join(ROOT, path);
  if (file.endsWith("/")) file += "index.html";
  if (!file.startsWith(ROOT) || !existsSync(file)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { "content-type": TYPES[extname(file)] || "application/octet-stream" }).end(readFileSync(file));
}).listen(0, "127.0.0.1");
await new Promise((r) => server.once("listening", r));
const port = server.address().port;

// CI's Ubuntu blocks Chrome's sandbox (unprivileged user namespaces), so it runs without it there.
const sandbox = process.env.CI ? ["--no-sandbox"] : [];
const chrome = spawn(CHROME, [...sandbox, "--headless=new", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check",
  `--user-data-dir=${mkdtempSync(join(tmpdir(), "diykey-e2e-"))}`, "--window-size=1400,2400", "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
const wsUrl = await new Promise((resolve, reject) => {
  let err = "";
  chrome.stderr.on("data", (d) => { err += d; const m = err.match(/DevTools listening on (ws:\/\/\S+)/); if (m) resolve(m[1]); });
  chrome.on("exit", () => reject(new Error("Chrome exited: " + err.slice(-400))));
});

// A minimal DevTools client over the browser's WebSocket, one page session.
const ws = new WebSocket(wsUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let nextId = 0;
const waiting = new Map();
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && waiting.has(m.id)) { const { resolve, reject } = waiting.get(m.id); waiting.delete(m.id); m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result); }
});
const cdp = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const id = ++nextId;
  waiting.set(id, { resolve, reject });
  ws.send(JSON.stringify({ id, method, params, sessionId }));
});
const { targetId } = await cdp("Target.createTarget", { url: "about:blank" });
const { sessionId } = await cdp("Target.attachToTarget", { targetId, flatten: true });
const page = (method, params) => cdp(method, params, sessionId);
const evaluate = async (expression) => (await page("Runtime.evaluate", { expression, awaitPromise: true, userGesture: true, returnByValue: true })).result.value;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (expression, what, ms = 8000) => {
  for (const end = Date.now() + ms; Date.now() < end; await sleep(100)) if (await evaluate(expression)) return;
  throw new Error("timed out waiting for " + what + "; status: " + await evaluate('document.getElementById("send-status").textContent'));
};
const press = async (key, code = key) => {
  const text = key.length === 1 ? key : key === "Enter" ? "\r" : undefined;
  await page("Input.dispatchKeyEvent", { type: "keyDown", key, code, text, windowsVirtualKeyCode: key === "Enter" ? 13 : key === "Backspace" ? 8 : 0 });
  await page("Input.dispatchKeyEvent", { type: "keyUp", key, code });
  await sleep(60);
};

let failed = 0;
const check = (ok, what) => { console.log((ok ? "ok   " : "FAIL ") + what); if (!ok) failed++; };
try {
  await page("WebAuthn.enable", {});
  await page("WebAuthn.addVirtualAuthenticator", { options: { protocol: "ctap2", transport: "internal", hasResidentKey: true,
    hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  await page("Runtime.enable", {});
  const errors = [];
  ws.addEventListener("message", (e) => { const m = JSON.parse(e.data); if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text); });
  await page("Page.navigate", { url: process.env.E2E_URL || `http://localhost:${port}/web/` });
  await until('document.getElementById("keypanel").textContent.includes("emulator (WebAssembly)")', "the emulator");
  check(true, "the emulator loaded (WebAssembly, under the page's CSP)");

  const button = (text) => evaluate(`[...document.querySelectorAll("#keypanel button")].find(b => b.textContent === ${JSON.stringify(text)}).click()`);
  const address = () => evaluate('document.querySelector("#kp-body .kc-addr code")?.textContent');
  check(await evaluate('document.getElementById("keypanel").classList.contains("open")'), "with no passkey, the key panel is open");
  await button("Make a passkey");
  await until('document.querySelector("#kp-body .kc-addr code")', "the passkey");
  const made = await address();
  check(/^0x[0-9a-fA-F]{40}$/.test(made), "a passkey made; its Safe signer address: " + made);
  check(await evaluate('!document.getElementById("keypanel").classList.contains("open") && document.getElementById("kp-tab").textContent.includes("' + "…" + '")'),
    "the panel folded to its tab: the account's blockie and short address");

  // A passkey made before: forget it here, find it again, and its address comes back from two signatures.
  await button("Forget this passkey");
  check(await address() === undefined, "forgotten: no key in the page");
  await button("Use one I made before");
  await until('[...document.querySelectorAll("#keypanel button")].some(b => b.textContent === "Confirm")', "the passkey found");
  await button("Confirm");
  await until('document.querySelector("#kp-body .kc-addr code")', "the passkey loaded");
  check(await address() === made, "loaded again from two signatures: the same address");

  // Reload with the passkey saved: the page hands it to the emulator first, and must still start.
  await page("Page.reload", {});
  await until('document.getElementById("keypanel").textContent.includes("emulator (WebAssembly)")', "the emulator after a reload");
  check(!errors.length && await evaluate('document.getElementById("pc-title").textContent.startsWith("This page")') && await address() === made,
    "reloaded with the passkey saved: the page started, the same address" + (errors.length ? "; " + errors.join("; ") : ""));

  // An honest claim: page to the hash on the drawn Cardputer's own keys, then its ok key signs.
  await evaluate('document.getElementById("send").click()');
  await until('document.getElementById("c-verdict").classList.contains("match")', "the emulator's hash");
  check(true, "the emulator worked out the same hash as the page");
  for (let i = 0; i < 4; i++) { await evaluate('document.querySelector(\'#cp-keys [data-live="right"]\').click()'); await sleep(60); }
  await evaluate('document.querySelector(\'#cp-keys [data-live="enter"]\').click()');
  await until('document.querySelector(".anat h4")', "the signature", 10000);
  check(await evaluate('document.querySelector(".anat h4").textContent.startsWith("Signed, and checked")'), "paged and signed on the drawn Cardputer's keys: the passkey signed, and the page verified it");
  check(await evaluate('[...document.querySelectorAll(".checks li")].every(li => li.classList.contains("yes"))'),
    "every check passed: " + await evaluate('[...document.querySelectorAll(".checks li")].map(li => li.textContent).join(", ")'));
  check(await evaluate('document.getElementById("log").textContent.includes("= signed")'), "the emulator says = signed");
  check(await evaluate('/^0x0{62}80/.test(document.querySelector(".safe-enc code").textContent)'), "the Safe encoding starts with its first offset, 0x80");
  if (process.argv[2]) {
    const { data } = await page("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
    writeFileSync(process.argv[2], Buffer.from(data, "base64"));
  }
  await press("Enter", "Enter"); // back to idle

  // A wrong claim: the hash page is red, and Enter must not sign.
  await evaluate('const t = document.getElementById("tamper"); t.checked = true; t.dispatchEvent(new Event("change"))');
  await evaluate('document.getElementById("log").replaceChildren()');
  await evaluate('document.getElementById("send").click()');
  await until('document.getElementById("c-verdict").classList.contains("differ")', "the mismatch");
  for (let i = 0; i < 4; i++) await press("/", "Slash"); // the computer's keyboard this time
  await press("Enter", "Enter");
  await sleep(400);
  check(!(await evaluate('document.getElementById("log").textContent.includes("= sign ")')), "a wrong claim: Enter on the red hash page doesn't sign");
  await press("Backspace", "Backspace");
  check(await evaluate('document.getElementById("log").textContent.includes("= rejected")'), "Del (Backspace) rejects it");
  check(errors.length === 0, "no errors in the page" + (errors.length ? ": " + errors.join("; ") : ""));

  // The key ceremony over USB, from a pretend Cardputer: a Web Serial port made in the page, which
  // answers ping and log with the lines of the rehearsal on a virtual chip (core/examples/ceremony.rs).
  // The page checks them as it checks a real chip's, and its panel shows each check passed.
  const rehearsal = readFileSync(join(ROOT, "web/screens/ceremony-rehearsal.txt"), "utf8");
  await page("Page.enable", {});
  await page("Page.addScriptToEvaluateOnNewDocument", { source: `(() => {
    const lines = ${JSON.stringify(rehearsal)};
    let push, buf = "";
    const readable = new ReadableStream({ start(c) { push = (t) => c.enqueue(new TextEncoder().encode(t)); } });
    const writable = new WritableStream({ write(chunk) {
      buf += new TextDecoder().decode(chunk);
      for (let i; (i = buf.indexOf("\\n")) >= 0; buf = buf.slice(i + 1)) {
        const l = buf.slice(0, i).trim();
        if (l === "ping") push("= pong diykey-ceremony\\n");
        if (l === "log") push("diykey-ceremony: commit " + "0".repeat(40) + "\\n" + lines + "= log end\\n");
      }
    } });
    const port = { readable, writable, open: async () => {}, close: async () => {}, getInfo: () => ({ usbVendorId: 0x303a }) };
    const serial = new EventTarget();
    serial.requestPort = async () => port;
    serial.getPorts = async () => [port];
    Object.defineProperty(navigator, "serial", { value: serial });
  })();` });
  await page("Page.navigate", { url: (process.env.E2E_URL || `http://localhost:${port}/web/`) + "index.html#usb" });
  await until('document.querySelectorAll("#txs li").length && document.body.classList.contains("is-emu") === false && !document.getElementById("connect").hidden', "USB mode");
  await evaluate('document.getElementById("connect").click()');
  await until('!document.getElementById("cer").hidden && document.querySelectorAll("#cer-checks li").length === 9', "the ceremony panel", 10000)
    .catch(async (e) => { console.log(await evaluate('document.getElementById("log").textContent.slice(-1500)')); throw e; });
  check(await evaluate('[...document.querySelectorAll("#cer-checks li")].every(li => li.classList.contains("yes"))'),
    "the ceremony from a pretend Cardputer: " + await evaluate('[...document.querySelectorAll("#cer-checks li")].map(li => li.textContent.split(" · ")[0]).join(", ")'));
  check(await evaluate('document.getElementById("cer-title").textContent === "Done: a sealed key"'), "the panel says the key is sealed");
  if (await evaluate('fetch("firmware/firmware.json").then(r => r.ok)')) {
    await evaluate('document.querySelector(\'#fw-pick [data-img="ceremony"]\').click()');
    check(await evaluate('document.getElementById("fw-info").textContent.includes("diykey-ceremony.bin") && document.getElementById("flash-btn").textContent === "Flash the key ceremony"'),
      "the key ceremony's image, picked to flash, with its SHA-256");
  }
  check(errors.length === 0, "no errors in the page" + (errors.length ? ": " + errors.join("; ") : ""));

  // The key ceremony's page, on a phone: every picture loads under its CSP, and nothing scrolls sideways.
  await page("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
  await page("Page.navigate", { url: new URL("ceremony.html", process.env.E2E_URL || `http://localhost:${port}/web/`).href });
  await until('document.querySelector(".cer-hero h1") && document.readyState === "complete"', "the ceremony page");
  check(await evaluate("document.images.length > 15 && [...document.images].every(i => i.complete && i.naturalWidth > 0)"),
    `the ceremony page: all ${await evaluate("document.images.length")} pictures load, under its CSP`);
  check(await evaluate("document.documentElement.scrollWidth <= innerWidth"), "the ceremony page fits a phone's width");
} catch (e) {
  failed++;
  console.log("FAIL " + e.message);
} finally {
  chrome.kill();
  server.close();
  ws.close();
}
console.log(failed ? `${failed} failed` : "ok: the emulator and a passkey, end to end");
process.exit(failed ? 1 : 0);
