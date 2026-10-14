// The page: demo transactions, sent to the Cardputer over Web Serial, and the firmware flashed with
// esptool-js. Everything it loads is in this folder or the repository's vectors/.
import { canvas as blockie } from "./lib/blockies.js";
import { fields, safeTxHash, tamper as tampered, txLine } from "./lib/safe.js";

const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};
const ESPRESSIF = 0x303a;
const CHAINS = {
  1: ["Ethereum", "rgb(98 126 234)"], 8453: ["Base", "rgb(0 82 255)"], 84532: ["Base Sepolia", "rgb(150 90 255)"],
  11155111: ["Sepolia", "rgb(120 120 120)"], 31337: ["Local", "rgb(255 149 0)"],
};
const LEVEL = { ok: "Plain", warn: "Careful", red: "Red" };

const st = {
  vectors: [], reviews: {}, sel: null, page: 0, tamper: false, filter: "all",
  port: null, reader: null, reading: null, lineBuf: "",
  pending: null, // { name, claimed, device, pages, done }
  firmware: null, flashing: false,
};

// ------------------------------------------------------------------ the log
function log(kind, text) {
  const pre = $("log");
  const row = el("div", kind);
  const t = new Date().toLocaleTimeString([], { hour12: false });
  row.append(el("span", "t", t), document.createTextNode(text));
  pre.append(row);
  while (pre.childNodes.length > 500) pre.firstChild.remove();
  pre.scrollTop = pre.scrollHeight;
}

// ------------------------------------------------------------------ helpers
const short = (a) => a.slice(0, 6) + "…" + a.slice(-4);
function units(wei, dec = 18) {
  const n = BigInt(wei), base = 10n ** BigInt(dec);
  const int = (n / base).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = (n % base).toString().padStart(dec, "0").replace(/0+$/, "");
  return frac ? `${int}.${frac}` : int;
}
function chainPill(id) {
  const [name, color] = CHAINS[id] || [`Chain ${id}`, "rgb(120 120 120)"];
  const p = el("span", "chain", name);
  p.style.background = color;
  if (id === 31337) p.style.color = "#1a1000";
  return p;
}
function addrTag(a, label) {
  const t = el("span", "chip-tag");
  t.append(blockie(a.toLowerCase(), 16), document.createTextNode((label ? label + " " : "") + short(a)));
  t.title = a;
  return t;
}
/** A hash with its last 8 characters (the verify code) picked out. */
function hashNode(h) {
  const f = document.createDocumentFragment();
  f.append(document.createTextNode(h.slice(0, -8)), el("span", "tail", h.slice(-8)));
  return f;
}

// ------------------------------------------------------------------ the list
function renderList() {
  const ul = $("txs");
  ul.replaceChildren();
  for (const v of st.vectors) {
    const r = st.reviews[v.name];
    if (st.filter !== "all" && r.level !== st.filter) continue;
    const li = el("li", `tx lv-${r.level}${st.sel === v ? " sel" : ""}`);
    li.tabIndex = 0;
    li.setAttribute("role", "button");
    const main = el("div", "tx-main");
    main.append(el("div", "tx-sum", r.summary));
    const meta = el("div", "tx-meta");
    meta.append(chainPill(v.chainId), el("code", "", v.name));
    main.append(meta);
    li.append(el("span", "tx-bar"), main, blockie(v.tx.to.toLowerCase(), 22));
    li.addEventListener("click", () => select(v));
    li.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(v); } });
    ul.append(li);
  }
}

// ------------------------------------------------------------------ the detail
function select(v) {
  st.sel = v;
  st.page = 0;
  st.pending = null;
  renderList();
  renderDetail();
  resetSend();
}

function renderDetail() {
  const v = st.sel, r = st.reviews[v.name], f = fields(v), h = safeTxHash(f);
  st.hash = h;

  const chips = $("d-chips");
  chips.replaceChildren(el("span", `badge badge-${r.level}`, LEVEL[r.level]), chainPill(v.chainId), addrTag(v.safe, "Safe"),
    el("span", "chip-tag plain", `nonce #${f.nonce}`));
  $("d-title").textContent = r.summary;
  $("d-desc").textContent = v.description || "";

  const items = $("d-items");
  items.replaceChildren();
  for (const [title, level] of r.items) {
    const li = el("li");
    li.append(el("span", `badge badge-${level}`, title), el("span", "lvl", LEVEL[level]));
    items.append(li);
  }

  const dl = $("d-fields");
  dl.replaceChildren();
  const row = (k, ...nodes) => { const dd = el("dd"); dd.append(...nodes); dl.append(el("dt", "", k), dd); };
  const data = f.data === "0x" ? 0 : (f.data.length - 2) / 2;
  row("chain", el("span", "", `${(CHAINS[v.chainId] || ["?"])[0]} (${f.chain})`));
  row("safe", blockie(f.safe.toLowerCase(), 18), el("code", "", f.safe));
  row("to", blockie(f.to.toLowerCase(), 18), el("code", "", f.to));
  row("value", el("span", "", `${units(f.value)} ETH`));
  row("data", el("span", "", data ? `${data} bytes, selector ${f.data.slice(0, 10)}` : "none"));
  row("operation", f.op === "1" ? el("span", "red", "DELEGATECALL") : el("span", "", "call"));
  row("nonce", el("code", "", f.nonce));
  if (f.safetxgas !== "0" || f.basegas !== "0" || f.gasprice !== "0") {
    row("gas", el("code", "", `safeTxGas ${f.safetxgas} · baseGas ${f.basegas} · gasPrice ${f.gasprice}`));
  }

  $("a-chain").textContent = f.chain;
  $("a-safe").textContent = short(f.safe);
  $("a-domain").textContent = h.domain;
  $("a-struct").textContent = h.struct;
  $("a-hash").replaceChildren(hashNode(h.hash));
  $("a-blockie").replaceChildren(blockie(h.hash, 34));
  const agrees = h.hash === v.safeTxHash.toLowerCase();
  $("a-note").textContent = agrees
    ? `Worked out in this browser just now; it agrees with the vector's published hash${v.name.startsWith("real") ? ", which came from Safe's transaction service for a transaction executed on Ethereum mainnet" : ""}. The Cardputer works it out again on its own.`
    : "This browser's hash differs from the vector's. Something is wrong with this page.";
  renderScreen();
  renderLine();
}

function screenPages() {
  return st.reviews[st.sel.name].pages;
}
function renderScreen() {
  const n = screenPages();
  const last = st.page === n - 1;
  const name = last && st.tamper ? `${st.sel.name}-tampered-${n}` : `${st.sel.name}-${st.page + 1}`;
  $("d-screen").src = `screens/${name}.png`;
  const dots = $("pg-dots");
  dots.replaceChildren();
  for (let i = 0; i < n; i++) {
    const b = el("button", (i === st.page ? "on " : "") + (i === n - 1 ? "hash" : ""));
    b.setAttribute("aria-label", i === n - 1 ? "The hash page" : `Page ${i + 1}`);
    b.addEventListener("click", () => { st.page = i; renderScreen(); });
    dots.append(b);
  }
  $("pg-prev").disabled = st.page === 0;
  $("pg-next").disabled = last;
}

function claimed() {
  return st.tamper ? tampered(st.hash.hash) : st.hash.hash;
}
function renderLine() {
  const line = txLine(fields(st.sel), claimed());
  const pre = $("d-line");
  pre.replaceChildren(el("span", "cmd", "tx"));
  for (const part of line.split(" ").slice(1)) {
    const [k, v] = part.split(/=(.*)/s);
    const s = el("span", k === "expect" ? `x${st.tamper ? " bad" : ""}` : "");
    s.append(document.createTextNode(" "), el("span", "k", k + "="), el("span", "v", v));
    pre.append(s);
  }
  // this page's claim
  const c = $("c-claim");
  c.classList.toggle("claim-bad", st.tamper);
  c.querySelector(".side-blockie").replaceChildren(blockie(claimed(), 34));
  c.querySelector(".code").textContent = claimed().slice(-8);
  c.querySelector(".side-full").replaceChildren(hashNode(claimed()));
}

// ------------------------------------------------------------------ the steps and the result
function steps(states) {
  for (const li of $("steps").children) {
    const s = states[li.dataset.step] || "";
    li.classList.remove("active", "done", "fail");
    if (s) li.classList.add(s);
    li.querySelector(".step-n").textContent = s === "done" ? "✓" : s === "fail" ? "!" : li.dataset.step;
  }
}
function status(text, kind = "") {
  const s = $("send-status");
  s.textContent = text;
  s.className = "send-status " + kind;
}
function flow(cls) {
  const f = $("flow");
  f.classList.remove("tx", "rx");
  if (cls) f.classList.add(cls);
}
function resetSend() {
  steps({});
  flow();
  const d = $("c-device");
  d.className = "side side-empty";
  d.querySelector(".side-blockie").replaceChildren();
  d.querySelector(".code").textContent = "— — — —";
  d.querySelector(".side-full").textContent = "send it to find out";
  const vd = $("c-verdict");
  vd.className = "verdict";
  vd.querySelector(".verdict-icon").textContent = "?";
  vd.querySelector(".verdict-text").textContent = "waiting";
  status(st.port ? "Ready. The Cardputer should show its idle screen." : "Connect the Cardputer to send.");
  $("flow-screen").setAttribute("href", "screens/idle.png");
  updateButtons();
}

function showDeviceHash(h) {
  const p = st.pending;
  p.device = h;
  const same = h === p.claimed;
  const d = $("c-device");
  d.className = "side " + (same ? "match" : "differ");
  d.querySelector(".side-blockie").replaceChildren(blockie(h, 34));
  d.querySelector(".code").textContent = h.slice(-8);
  d.querySelector(".side-full").replaceChildren(hashNode(h));
  const vd = $("c-verdict");
  vd.className = "verdict " + (same ? "match" : "differ");
  vd.querySelector(".verdict-icon").textContent = same ? "=" : "≠";
  vd.querySelector(".verdict-text").textContent = same ? "the same hash" : "not the same";
  const honest = h === st.hash.hash;
  steps({ 1: "done", 2: honest ? "done" : "fail", 3: "active" });
  flow();
  if (same) {
    status(`The Cardputer worked out the same hash. Page through it on the device: ${p.pages || "its"} pages. Del rejects.`, "good");
  } else if (honest) {
    status("The Cardputer worked out the real hash and saw this page's claim is wrong. Its hash page is red: it says don't sign. Del rejects.", "bad");
  } else {
    status("The Cardputer's hash is different from the one this browser worked out. Don't trust either until you know why.", "bad");
  }
  // the device's screen, in the picture: its first page, then the hash page once it's shown
  $("flow-screen").setAttribute("href", `screens/${p.name}-1.png`);
}

// ------------------------------------------------------------------ lines from the Cardputer
function onLine(line) {
  line = line.replace(/\r$/, "");
  if (!line.trim()) return;
  const reply = line.startsWith("= ");
  log(reply ? (/^= (error|mismatch|busy)/.test(line) ? "in bad" : "in") : "dev", line);

  let m;
  if ((m = line.match(/^cardputer-signer: (.+)$/)) || (m = line.match(/^= pong (cardputer-signer .+)$/))) {
    $("t-fw").textContent = m[1].replace(/^cardputer-signer /, "").replace(/^m(\d)$/, "milestone $1");
    connState("on", "Connected");
  }
  if ((m = line.match(/^chip: serial \[([0-9a-f, ]+)\] revision \[([0-9a-f, ]+)\] config locked (\w+) data locked (\w+) counter0 (\d+)/))) {
    const serial = m[1].replace(/[ ,]/g, ""), rev = m[2].replace(/[ ,]/g, "");
    const model = rev.endsWith("6002") ? "ATECC608A" : rev.endsWith("6003") ? "ATECC608B" : "ATECC608";
    const v = $("t-chip");
    v.replaceChildren(blockie(serial, 26), el("span", "", model), el("code", "", serial.match(/.{1,4}/g).join(" ")));
    $("t-locks").textContent = `config ${m[3] === "true" ? "locked" : "open"} · data ${m[4] === "true" ? "locked" : "open"}`;
    $("t-counter").textContent = m[5];
    $("flow").classList.add("haschip");
    pulseChip();
  } else if (line.startsWith("chip: no answer")) {
    $("t-chip").replaceChildren(el("span", "", "No chip at 0x60"), el("span", "muted", "check the Grove wiring"));
    $("flow").classList.remove("haschip");
  } else if (line.startsWith("chip: ")) {
    $("t-chip").replaceChildren(el("span", "", line.slice(6)));
  }

  const p = st.pending;
  if (!p || p.done) return;
  if ((m = line.match(/^= hash (0x[0-9a-fA-F]+)/))) {
    clearTimeout(p.timer);
    if (m[1].length !== 66) {
      status("The hash came back short: the USB link dropped some of it. Send again.", "bad");
      return;
    }
    showDeviceHash(m[1].toLowerCase());
  } else if ((m = line.match(/^tx: (.*) \((\d+) pages\)$/))) {
    p.pages = Number(m[2]);
    if (p.device && p.device === p.claimed) {
      status(`The Cardputer worked out the same hash. Page through it on the device: ${p.pages} pages. Del rejects.`, "good");
    }
  } else if (line === "= rejected") {
    p.done = true;
    steps({ 1: "done", 2: p.device === st.hash.hash ? "done" : "fail", 3: "done" });
    status("Rejected on the Cardputer. Nothing was signed; the key wasn't used.", "");
    $("flow-screen").setAttribute("href", "screens/rejected.png");
    updateButtons();
  } else if (line === "= busy") {
    clearTimeout(p.timer);
    p.done = true;
    steps({ 1: "fail" });
    flow();
    status("A transaction is already on the Cardputer's screen. Press Del there to reject it, then send again.", "bad");
    updateButtons();
  } else if ((m = line.match(/^= error (.*)$/))) {
    clearTimeout(p.timer);
    p.done = true;
    steps({ 1: "fail" });
    flow();
    status(`The Cardputer refused it: ${m[1]}.`, "bad");
    updateButtons();
  }
}

function pulseChip() {
  const f = $("flow");
  f.classList.add("chip");
  setTimeout(() => f.classList.remove("chip"), 1400);
}

// ------------------------------------------------------------------ Web Serial
function connState(kind, text) {
  $("conn-pill").className = `pill pill-${kind}`;
  $("conn-text").textContent = text;
}
function updateButtons() {
  const open = !!st.port && !st.flashing;
  const busy = st.pending && !st.pending.done;
  $("send").disabled = !open || !st.sel || busy;
  $("raw-send").disabled = !open;
  $("connect").textContent = st.port ? "Disconnect" : "Connect over USB";
  $("connect").disabled = st.flashing;
  $("flash-btn").disabled = !st.firmware || !$("fw-ok").checked || st.flashing || !("serial" in navigator);
}

async function openPort(port) {
  await port.open({ baudRate: 115200 });
  st.port = port;
  st.lineBuf = "";
  connState("on", "Connected");
  log("sys", "port open");
  if (!$("flow").classList.contains("haschip")) {
    // it prints the chip only at start-up, or on Enter at its idle screen; opening the port doesn't reset it
    $("t-chip").replaceChildren(el("span", "muted", "press Enter on the Cardputer's idle screen to read it"));
  }
  st.reading = readLoop(port);
  updateButtons();
  resetSend();
  await send("ping");
}

async function readLoop(port) {
  const dec = new TextDecoder();
  while (port.readable && st.port === port) {
    st.reader = port.readable.getReader();
    try {
      for (;;) {
        const { value, done } = await st.reader.read();
        if (done) break;
        st.lineBuf += dec.decode(value, { stream: true });
        let i;
        while ((i = st.lineBuf.indexOf("\n")) >= 0) {
          const line = st.lineBuf.slice(0, i);
          st.lineBuf = st.lineBuf.slice(i + 1);
          onLine(line);
        }
      }
    } catch (e) {
      if (st.port === port) log("sys", `read: ${e.message}`);
    } finally {
      st.reader.releaseLock();
      st.reader = null;
    }
  }
}

async function closePort() {
  const port = st.port;
  if (!port) return port;
  st.port = null;
  try { await st.reader?.cancel(); } catch { /* already gone */ }
  try { await st.reading; } catch { /* done */ }
  try { await port.close(); } catch { /* unplugged */ }
  connState("off", "Not connected");
  log("sys", "port closed");
  updateButtons();
  return port;
}

async function send(line) {
  if (!st.port?.writable) return;
  const w = st.port.writable.getWriter();
  try {
    await w.write(new TextEncoder().encode(line + "\n"));
    log("out", "> " + (line.length > 160 ? line.slice(0, 160) + "…" : line));
  } finally {
    w.releaseLock();
  }
}

async function connect() {
  if (st.port) { await closePort(); resetSend(); return; }
  try {
    const port = await navigator.serial.requestPort({ filters: [{ usbVendorId: ESPRESSIF }] });
    await openPort(port);
  } catch (e) {
    if (e.name !== "NotFoundError") log("sys", `connect: ${e.message}`);
  }
}

async function sendTx() {
  const v = st.sel;
  const p = { name: v.name, claimed: claimed(), device: null, pages: 0, done: false };
  st.pending = p;
  resetSend();
  steps({ 1: "active" });
  flow("tx");
  status("Sending the fields…");
  await send(txLine(fields(v), p.claimed));
  if (p.done || p.device) return; // the answer came back before the write finished
  steps({ 1: "done", 2: "active" });
  flow("rx");
  status("Waiting for the Cardputer's hash…");
  p.timer = setTimeout(() => {
    if (p.device || p.done) return;
    p.done = true;
    steps({ 1: "done", 2: "fail" });
    flow();
    status("No answer in 4 seconds. Is the signer firmware on the board? The log below shows what came back.", "bad");
    updateButtons();
  }, 4000);
}

// ------------------------------------------------------------------ flashing
async function loadFirmware() {
  const dl = $("fw-info");
  try {
    const info = await (await fetch("firmware/firmware.json", { cache: "no-store" })).json();
    st.firmware = info;
    dl.replaceChildren();
    const row = (k, ...v) => { const dd = el("dd"); dd.append(...v); dl.append(el("dt", "", k), dd); };
    row("Image", el("code", "", info.file), document.createTextNode(` · ${info.size.toLocaleString()} bytes at 0x0`));
    const sha = el("code");
    sha.append(document.createTextNode(info.sha256.slice(0, 56)), el("b", "", info.sha256.slice(56)));
    row("SHA-256", sha);
    row("Built from", el("code", "", info.commit), document.createTextNode(` · ${info.built}`),
      ...(info.dirty ? [el("span", "warn", " · with uncommitted changes")] : []));
    row("For", document.createTextNode(`${info.chip.toUpperCase().replace(/^ESP32/, "ESP32-")}, ${info.flashSize} flash (Cardputer v1.1)`));
  } catch {
    dl.replaceChildren(el("dt", "", "Image"), el("dd", "warn", "Not built yet: run web/build.sh, then reload."));
  }
  updateButtons();
}

async function sha256hex(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(d, (x) => x.toString(16).padStart(2, "0")).join("");
}

function stage(text, frac) {
  $("fw-progress").hidden = false;
  $("fw-stage").textContent = text;
  if (frac !== undefined) $("fw-bar").style.width = `${Math.round(frac * 100)}%`;
}

async function flash() {
  st.flashing = true;
  updateButtons();
  connState("busy", "Flashing");
  let transport;
  try {
    stage("Checking the image…", 0);
    const data = new Uint8Array(await (await fetch(`firmware/${st.firmware.file}`, { cache: "no-store" })).arrayBuffer());
    const got = await sha256hex(data);
    if (got !== st.firmware.sha256) throw new Error(`the image's SHA-256 is ${got}, not the one in firmware.json`);
    log("sys", `image ok: ${data.length} bytes, sha256 ${got}`);

    const port = (await closePort()) || await navigator.serial.requestPort({ filters: [{ usbVendorId: ESPRESSIF }] });
    stage("Loading the flasher…");
    const { ESPLoader, Transport } = await import("./vendor/esptool-js/bundle.js");
    transport = new Transport(port, false);
    const terminal = { clean() {}, writeLine: (s) => log("sys", s), write: (s) => { if (s.trim()) log("sys", s); } };
    const loader = new ESPLoader({ transport, baudrate: 115200, romBaudrate: 115200, terminal, debugLogging: false });
    stage("Connecting to the bootloader…");
    const chip = await loader.main();
    if (!/ESP32-S3/i.test(chip)) throw new Error(`this is an ${chip}, not an ESP32-S3`);
    stage(`Writing to the ${chip}…`, 0);
    await loader.writeFlash({
      fileArray: [{ data, address: 0 }], flashMode: "keep", flashFreq: "keep", flashSize: "keep",
      eraseAll: false, compress: true,
      reportProgress: (_i, written, total) => stage(`Writing… ${Math.round((written / total) * 100)}%`, written / total),
    });
    stage("Restarting the Cardputer…", 1);
    await loader.after("hard_reset");
    await transport.disconnect();
    transport = null;
    stage("Done. The signer is on the board.", 1);
    st.flashing = false;
    try { await openPort(port); } catch { log("sys", "reconnect after flashing: press Connect"); connState("off", "Not connected"); }
  } catch (e) {
    stage(`Stopped: ${e.message}`);
    log("sys", `flash: ${e.message}`);
    connState("off", "Not connected");
    try { await transport?.disconnect(); } catch { /* gone */ }
  } finally {
    st.flashing = false;
    updateButtons();
  }
}

// ------------------------------------------------------------------ start
async function main() {
  if (!("serial" in navigator)) $("no-serial").hidden = false;
  const [vec, reviews] = await Promise.all([
    fetch("../vectors/safe_tx.json").then((r) => r.json()),
    fetch("screens/reviews.json").then((r) => r.json()).catch(() => null),
  ]);
  if (!reviews) {
    $("detail").replaceChildren(el("p", "warn", "The screens aren't built yet: run web/build.sh, then reload."));
    return;
  }
  loadFirmware();
  st.vectors = vec.vectors;
  st.reviews = reviews;
  select(st.vectors.find((v) => v.name === "eth-send-base-sepolia") || st.vectors[0]);

  $("filters").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    st.filter = b.dataset.f;
    for (const x of $("filters").children) x.classList.toggle("on", x === b);
    renderList();
  });
  $("pg-prev").addEventListener("click", () => { st.page = Math.max(0, st.page - 1); renderScreen(); });
  $("pg-next").addEventListener("click", () => { st.page = Math.min(screenPages() - 1, st.page + 1); renderScreen(); });
  $("tamper").addEventListener("change", (e) => {
    st.tamper = e.target.checked;
    st.page = screenPages() - 1; // show what the device does with it: the hash page
    st.pending = null;
    renderScreen();
    renderLine();
    resetSend();
  });
  $("connect").addEventListener("click", connect);
  $("send").addEventListener("click", sendTx);
  $("raw").addEventListener("submit", async (e) => {
    e.preventDefault();
    const line = $("raw-line").value.trim();
    if (line) { await send(line); $("raw-line").value = ""; }
  });
  $("fw-ok").addEventListener("change", updateButtons);
  $("flash-btn").addEventListener("click", flash);
  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, textarea")) return;
    if (e.key === "ArrowRight") $("pg-next").click();
    if (e.key === "ArrowLeft") $("pg-prev").click();
  });
  if ("serial" in navigator) {
    navigator.serial.addEventListener("disconnect", (e) => {
      if (e.target === st.port || e.port === st.port) { st.port = null; connState("off", "Unplugged"); log("sys", "unplugged"); updateButtons(); }
    });
    // a port this page was given before: reopen it without asking
    const known = (await navigator.serial.getPorts()).filter((p) => p.getInfo().usbVendorId === ESPRESSIF);
    if (known.length === 1) openPort(known[0]).catch((e) => log("sys", `reopen: ${e.message}`));
  }
}
main();
