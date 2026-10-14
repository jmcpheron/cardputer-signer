// The page: demo transactions, sent to a signer, and the firmware flashed with esptool-js. The
// signer is either the emulator (the signer's core in WebAssembly, with a passkey as its key) or a
// Cardputer over Web Serial. Both answer in the same lines (core/src/proto.rs), so one handler
// reads both. Everything the page loads is in this folder or the repository's vectors/.
import { canvas as blockie } from "./lib/blockies.js";
import { load as loadEmu } from "./lib/emu.js";
import * as passkey from "./lib/passkey.js";
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
  mode: "emu", vectors: [], reviews: {}, sel: null, page: 0, tamper: false, filter: "all",
  port: null, reader: null, reading: null, lineBuf: "",
  emu: null, key: passkey.saved(), signing: false,
  pending: null, // { name, claimed, device, pages, done, signed }
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
const emuMode = () => st.mode === "emu";
const signerName = () => (emuMode() ? "the emulator" : "the Cardputer");

// ------------------------------------------------------------------ the mode
const LEDE = {
  emu: "Send a demo Safe transaction to the emulator: the Cardputer's own code, running in this page, with your passkey as its key. It rebuilds the transaction's hash from its fields, shows you what it does, and only compares the hash this page claims. Page through it, then Enter on the last page signs with your passkey. A wrong claim turns it red, and it won't sign.",
  usb: "Send a demo Safe transaction to your Cardputer over USB. The Cardputer rebuilds the transaction's hash from its fields, shows you what it does, and only compares the hash this page claims. A wrong claim turns it red.",
};

function setMode(m) {
  st.mode = m;
  const emu = emuMode();
  for (const b of $("modes").children) b.classList.toggle("on", b.dataset.m === m);
  document.body.classList.toggle("is-emu", emu);
  $("lede").textContent = LEDE[m];
  $("usb-label").textContent = emu ? "in this page" : "USB";
  $("grove-label").textContent = emu ? "WebAuthn" : "Grove";
  $("card-label").textContent = emu ? "Emulator" : "Cardputer";
  $("chip-text").textContent = emu ? "P-256" : "608";
  $("chip-label").textContent = emu ? "Your passkey" : "ATECC608";
  $("chip-sub").textContent = emu ? "a virtual ATECC608: P-256, can't be exported" : "the key, made inside, never leaves";
  $("step4-title").textContent = emu ? "Your passkey signs" : "The chip signs";
  $("step4-sub").textContent = emu ? "Enter on the last page: the passkey signs the emulator's hash, and nothing else."
    : "Milestone 3: the chip signs that hash, and nothing else.";
  $("steps").lastElementChild.classList.toggle("future", !emu);
  $("send-title").textContent = `Send it to ${signerName()}`;
  $("send").textContent = emu ? "Send to the emulator" : "Send to the Cardputer";
  $("c-device-k").textContent = emu ? "The emulator worked out" : "The Cardputer worked out";
  $("connect").hidden = emu;
  $("emu-screen").hidden = !emu;
  $("d-screen").hidden = emu;
  $("emu-keys").hidden = !emu;
  $("pager").hidden = emu;
  $("d-caption").textContent = emu
    ? "Live: the signer's core in WebAssembly. Keys: , and / page, Enter, Del (or Backspace)."
    : "Drawn on the laptop by the firmware's own code, pixel for pixel.";
  $("pk").hidden = !emu;
  $("sig").hidden = emu;
  for (const id of ["t-fw", "t-chip", "t-locks", "t-counter"]) $(id).textContent = "—";
  $("flow").classList.remove("haschip");
  st.pending = null;
  if (emu) {
    connState("on", "Emulator");
    if (st.emu) emuLines(st.emu.boot());
    renderPasskey();
    drawEmu();
  } else {
    connState(st.port ? "on" : "off", st.port ? "Connected" : "Not connected");
    $("flow-screen").setAttribute("href", "screens/idle.png");
    if (st.port) send("ping");
  }
  resetSend();
  if (st.sel) renderScreen();
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
    ? `Worked out in this browser just now; it agrees with the vector's published hash${v.name.startsWith("real") ? ", which came from Safe's transaction service for a transaction executed on Ethereum mainnet" : ""}. The signer works it out again on its own.`
    : "This browser's hash differs from the vector's. Something is wrong with this page.";
  renderScreen();
  renderLine();
}

function screenPages() {
  return st.reviews[st.sel.name].pages;
}
function renderScreen() {
  if (emuMode()) return drawEmu();
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
  const c = $("c-claim");
  c.classList.toggle("claim-bad", st.tamper);
  c.querySelector(".side-blockie").replaceChildren(blockie(claimed(), 34));
  c.querySelector(".code").textContent = claimed().slice(-8);
  c.querySelector(".side-full").replaceChildren(hashNode(claimed()));
}

// ------------------------------------------------------------------ the emulator
function drawEmu() {
  if (!st.emu) return;
  const c = $("emu-screen");
  c.getContext("2d").putImageData(new ImageData(st.emu.pixels(), st.emu.w, st.emu.h), 0, 0);
  $("flow-screen").setAttribute("href", c.toDataURL());
}
/** Lines from the emulator, read like the Cardputer's. */
function emuLines(lines) {
  for (const l of lines) onLine(l);
  drawEmu();
}
function emuKey(name) {
  if (!st.emu || !emuMode() || st.signing) return;
  emuLines(st.emu.key(name));
}

// ------------------------------------------------------------------ the passkey
function renderPasskey() {
  const pk = $("pk");
  pk.replaceChildren();
  const head = el("div", "pk-head");
  const k = st.key;
  if (!k) {
    head.append(el("div", "pk-icon", "🔑"));
    const t = el("div");
    t.append(el("b", "", "Make a passkey to sign"),
      el("p", "", "A new passkey for this site, made by your phone or computer (Face ID, Touch ID, Windows Hello, or a security key). It is the emulator's key, in place of the ATECC608: P-256, made inside your authenticator, and it can't be exported."));
    head.append(t);
    const b = el("button", "btn btn-primary", "Make a passkey");
    b.addEventListener("click", makePasskey);
    head.append(b);
    pk.append(head);
    if (!window.PublicKeyCredential) pk.append(el("p", "pk-note bad", "This browser has no passkeys (WebAuthn)."));
    return;
  }
  head.append(blockie(k.id, 34));
  const t = el("div");
  t.append(el("b", "", `Passkey ${k.id.slice(0, 4)} ${k.id.slice(4, 8)}`), el("p", "", `P-256 public key x ${k.x.slice(0, 10)}…  y ${k.y.slice(0, 10)}…`));
  head.append(t);
  const f = el("button", "btn btn-ghost", "Forget");
  f.title = "Forget this passkey in this page. It stays in your keychain.";
  f.addEventListener("click", () => { passkey.forget(); st.key = null; if (st.emu) emuLines(st.emu.setPasskey(null)); renderPasskey(); });
  head.append(f);
  pk.append(head);
  pk.append(el("p", "pk-note", "Page through a transaction on the emulator; on the last page, Enter asks your passkey to sign the hash the emulator worked out."));
  if (st.lastSig) pk.append(sigAnatomy(st.lastSig));
}

async function makePasskey() {
  try {
    st.key = await passkey.create();
    log("sys", `passkey made: ${st.key.id.slice(0, 16)}…`);
    if (st.emu) emuLines(st.emu.setPasskey(st.key.id));
  } catch (e) {
    log("sys", `passkey: ${e.name}: ${e.message}`);
    status(e.name === "NotAllowedError" ? "No passkey made (cancelled)." : `Couldn't make a passkey: ${e.message}`, "bad");
  }
  renderPasskey();
}

/** The emulator asked to sign `hash`, the one it worked out. Runs inside the key press: Safari
 * shows a passkey prompt only from one. */
function startSign(hash) {
  const p = st.pending;
  if (!st.key || !p || hash !== p.device) {
    // never sign a hash the emulator didn't just show
    emuLines(st.emu.signResult(false));
    return;
  }
  st.signing = true;
  steps({ 1: "done", 2: "done", 3: "done", 4: "active" });
  flow("chip");
  status("Your passkey is asking: confirm to sign the emulator's hash.");
  passkey.sign(hash, st.key)
    .then((a) => passkey.verify({ ...a, x: passkey.fromHex(st.key.x), y: passkey.fromHex(st.key.y), hash,
      rpId: location.hostname, origin: location.origin }))
    .then((v) => {
      st.lastSig = { ...v, hash };
      log(v.ok ? "in" : "in bad", `passkey signature ${v.ok ? "verified" : "FAILED: " + v.checks.filter(([, ok]) => !ok).map(([k]) => k).join(", ")}`);
      st.signing = false;
      emuLines(st.emu.signResult(v.ok));
      renderPasskey();
    })
    .catch((e) => {
      log("sys", `passkey: ${e.name}: ${e.message}`);
      st.signing = false;
      emuLines(st.emu.signResult(false));
      status(e.name === "NotAllowedError" ? "Not signed: the passkey prompt was cancelled. Enter tries again; Del rejects." : `Not signed: ${e.message}`, "bad");
    });
}

function sigAnatomy(v) {
  const box = el("div", "anat");
  box.append(el("h4", "", v.ok ? "Signed, and checked in this page" : "The signature didn't check out"));
  const checks = el("ul", "checks");
  for (const [k, ok] of v.checks) checks.append(el("li", ok ? "yes" : "no", (ok ? "✓ " : "✗ ") + k));
  box.append(checks);
  const grid = el("dl", "anat-grid");
  const row = (k, node, cls) => { const dd = el("dd", cls); dd.append(node); grid.append(el("dt", "", k), dd); };
  const ch = el("code");
  const i = v.client.indexOf(v.challenge);
  ch.append(document.createTextNode(v.client.slice(0, i)), el("b", "", v.challenge), document.createTextNode(v.client.slice(i + v.challenge.length)));
  row("clientDataJSON", ch);
  row("challenge", el("code", "", `base64url of the emulator's hash ${v.hash.slice(0, 10)}…${v.hash.slice(-8)}`));
  row("flags", el("code", "", `0x${v.flags.toString(16).padStart(2, "0")}: ${v.flags & 1 ? "UP " : ""}${v.flags & 4 ? "UV " : ""}${v.flags & 8 ? "BE " : ""}${v.flags & 16 ? "BS (synced)" : ""}`));
  row("signed message", el("code", "", `authenticatorData ‖ sha256(clientDataJSON), ${v.message.length / 2} bytes`));
  row("r", el("code", "", v.r));
  row("s", el("code", "", v.s + "  (low-s)"));
  box.append(grid);
  if (v.safe) {
    const d = el("details", "safe-enc");
    d.append(el("summary", "", "As Safe's passkey signer reads it: abi.encode(authenticatorData, clientDataFields, r, s)"),
      el("code", "", v.safe), el("p", "fine", "Not yet checked against a Safe onchain. That comes with a Safe owner for the passkey."));
    box.append(d);
  }
  if (v.flags & 16) {
    box.append(el("p", "pk-note", "This passkey is synced (BS): its key is copied, encrypted, to your other devices. The chip's key never leaves the chip."));
  }
  return box;
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
  f.classList.remove("tx", "rx", "chip");
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
  if (emuMode()) status(st.emu ? "Ready: the emulator is on its idle screen." : "Loading the emulator…");
  else status(st.port ? "Ready. The Cardputer should show its idle screen." : "Connect the Cardputer to send.");
  if (!emuMode()) $("flow-screen").setAttribute("href", "screens/idle.png");
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
  const who = emuMode() ? "The emulator" : "The Cardputer";
  if (same) {
    status(`${who} worked out the same hash. Page through it ${emuMode() ? "with › or /" : "on the device"}: ${p.pages || "its"} pages. ${emuMode() && st.key ? "Enter on the last page signs. " : ""}Del rejects.`, "good");
  } else if (honest) {
    status(`${who} worked out the real hash and saw this page's claim is wrong. Its hash page is red: it says don't sign, and it won't. Del rejects.`, "bad");
  } else {
    status(`${who}'s hash is different from the one this browser worked out. Don't trust either until you know why.`, "bad");
  }
  if (!emuMode()) $("flow-screen").setAttribute("href", `screens/${p.name}-1.png`);
}

// ------------------------------------------------------------------ lines from the signer
function onLine(line) {
  line = line.replace(/\r$/, "");
  if (!line.trim()) return;
  const reply = line.startsWith("= ");
  log(reply ? (/^= (error|mismatch|busy|not signed)/.test(line) ? "in bad" : "in") : "dev", line);

  let m;
  if ((m = line.match(/^cardputer-signer: (.+)$/)) || (m = line.match(/^= pong (cardputer-signer .+)$/))) {
    const what = m[1].replace(/^cardputer-signer /, "");
    $("t-fw").textContent = what === "emulator" ? "emulator (WebAssembly)" : what.replace(/^m(\d)$/, "milestone $1").replace(/^milestone (\d)$/, "milestone $1");
    if (!emuMode()) connState("on", "Connected");
  }
  if ((m = line.match(/^chip: serial \[([0-9a-f, ]+)\] revision \[([0-9a-f, ]+)\] config locked (\w+) data locked (\w+) counter0 (\d+)/))) {
    const serial = m[1].replace(/[ ,]/g, ""), rev = m[2].replace(/[ ,]/g, "");
    const model = rev.endsWith("6002") ? "ATECC608A" : rev.endsWith("6003") ? "ATECC608B" : "ATECC608";
    $("t-chip").replaceChildren(blockie(serial, 26), el("span", "", model), el("code", "", serial.match(/.{1,4}/g).join(" ")));
    $("t-locks").textContent = `config ${m[3] === "true" ? "locked" : "open"} · data ${m[4] === "true" ? "locked" : "open"}`;
    $("t-counter").textContent = m[5];
    $("flow").classList.add("haschip");
    pulseChip();
  } else if ((m = line.match(/^chip: passkey ([0-9a-f]+)/))) {
    $("t-chip").replaceChildren(blockie(m[1], 26), el("span", "", "Passkey"), el("code", "", `${m[1].slice(0, 4)} ${m[1].slice(4, 8)} · P-256`));
    $("t-locks").textContent = "can't be exported";
    $("t-counter").textContent = st.lastSig ? String(st.lastSig.counter) : "—";
    $("flow").classList.add("haschip");
  } else if (line.startsWith("chip: no key")) {
    $("t-chip").replaceChildren(el("span", "", "No passkey yet"), el("span", "muted", "make one below to sign"));
    $("flow").classList.remove("haschip");
  } else if (line.startsWith("chip: no answer")) {
    $("t-chip").replaceChildren(el("span", "", "No chip at 0x60"), el("span", "muted", "check the Grove wiring"));
    $("flow").classList.remove("haschip");
  } else if (line.startsWith("chip: ")) {
    $("t-chip").replaceChildren(el("span", "", line.slice(6)));
  }

  const p = st.pending;
  if (!p || (p.done && !p.device)) return;
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
      status(`${emuMode() ? "The emulator" : "The Cardputer"} worked out the same hash. Page through it ${emuMode() ? "with › or /" : "on the device"}: ${p.pages} pages. ${emuMode() && st.key ? "Enter on the last page signs. " : ""}Del rejects.`, "good");
    }
  } else if ((m = line.match(/^= sign (0x[0-9a-f]{64})$/))) {
    startSign(m[1]);
  } else if (line === "= signed") {
    p.done = true;
    p.signed = true;
    steps({ 1: "done", 2: "done", 3: "done", 4: "done" });
    flow();
    status("Signed by your passkey over the emulator's hash, and the signature checked out in this page. Enter on the emulator goes back.", "good");
    updateButtons();
  } else if (line === "= not signed") {
    steps({ 1: "done", 2: "done", 3: "active" });
    flow();
  } else if (line === "= rejected") {
    p.done = true;
    steps({ 1: "done", 2: p.device === st.hash.hash ? "done" : "fail", 3: "done" });
    status(`Rejected on ${signerName()}. Nothing was signed; the key wasn't used.`, "");
    if (!emuMode()) $("flow-screen").setAttribute("href", "screens/rejected.png");
    updateButtons();
  } else if (line === "= busy") {
    clearTimeout(p.timer);
    p.done = true;
    steps({ 1: "fail" });
    flow();
    status(`A transaction is already on ${signerName()}'s screen. Press Del there to reject it, then send again.`, "bad");
    updateButtons();
  } else if ((m = line.match(/^= error (.*)$/))) {
    clearTimeout(p.timer);
    p.done = true;
    steps({ 1: "fail" });
    flow();
    status(`${emuMode() ? "The emulator" : "The Cardputer"} refused it: ${m[1]}.`, "bad");
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
  const ready = emuMode() ? !!st.emu : !!st.port && !st.flashing;
  const busy = st.pending && !st.pending.done;
  $("send").disabled = !ready || !st.sel || busy;
  $("raw-send").disabled = !ready;
  $("connect").textContent = st.port ? "Disconnect" : "Connect over USB";
  $("connect").disabled = st.flashing;
  $("flash-btn").disabled = !st.firmware || !$("fw-ok").checked || st.flashing || !("serial" in navigator);
}

async function openPort(port) {
  await port.open({ baudRate: 115200 });
  st.port = port;
  st.lineBuf = "";
  log("sys", "port open");
  st.reading = readLoop(port);
  if (!emuMode()) {
    connState("on", "Connected");
    if (!$("flow").classList.contains("haschip")) {
      // it prints the chip only at start-up, or on Enter at its idle screen; opening the port doesn't reset it
      $("t-chip").replaceChildren(el("span", "muted", "press Enter on the Cardputer's idle screen to read it"));
    }
    resetSend();
    await send("ping");
  }
  updateButtons();
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
          if (!emuMode()) onLine(line);
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
  if (!emuMode()) connState("off", "Not connected");
  log("sys", "port closed");
  updateButtons();
  return port;
}

/** One line to the signer: the emulator, or the Cardputer over USB. */
async function send(line) {
  const shown = "> " + (line.length > 160 ? line.slice(0, 160) + "…" : line);
  if (emuMode()) {
    if (!st.emu) return;
    log("out", shown);
    // after the caller has set up, as a Cardputer's reply comes after the write
    queueMicrotask(() => emuLines(st.emu.send(line)));
    return;
  }
  if (!st.port?.writable) return;
  const w = st.port.writable.getWriter();
  try {
    await w.write(new TextEncoder().encode(line + "\n"));
    log("out", shown);
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
  if (emuMode()) $("emu-screen").focus(); // so Enter and the page keys go to the emulator
  await send(txLine(fields(v), p.claimed));
  if (p.done || p.device) return; // the answer came back before the write finished
  steps({ 1: "done", 2: "active" });
  flow("rx");
  status(`Waiting for ${signerName()}'s hash…`);
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
    if (!emuMode()) connState("off", "Not connected");
    try { await transport?.disconnect(); } catch { /* gone */ }
  } finally {
    st.flashing = false;
    if (emuMode()) connState("on", "Emulator");
    updateButtons();
  }
}

// ------------------------------------------------------------------ start
const KEYMAP = { ",": "left", ArrowLeft: "left", "/": "right", ArrowRight: "right", ";": "up", ArrowUp: "up",
  ".": "down", ArrowDown: "down", Enter: "enter", Backspace: "del", Delete: "del", Escape: "del" };

async function main() {
  if (!("serial" in navigator)) {
    $("no-serial").hidden = false;
    $("modes").querySelector('[data-m="usb"]').disabled = true;
  }
  loadFirmware();
  const [vec, reviews, wasm] = await Promise.all([
    fetch("../vectors/safe_tx.json").then((r) => r.json()),
    fetch("screens/reviews.json").then((r) => r.json()).catch(() => null),
    fetch("emu/signer.wasm").then((r) => (r.ok ? r.arrayBuffer() : null)).catch(() => null),
  ]);
  if (!reviews) {
    $("detail").replaceChildren(el("p", "warn", "The screens aren't built yet: run web/build.sh, then reload."));
    return;
  }
  st.vectors = vec.vectors;
  st.reviews = reviews;
  if (wasm) {
    st.emu = await loadEmu(wasm);
    if (st.key) st.emu.setPasskey(st.key.id);
  } else {
    log("sys", "the emulator isn't built: run web/build.sh");
  }
  select(st.vectors.find((v) => v.name === "eth-send-base-sepolia") || st.vectors[0]);
  setMode(st.emu ? "emu" : "usb");

  $("modes").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (b && !b.disabled && b.dataset.m !== st.mode) setMode(b.dataset.m);
  });
  $("filters").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    st.filter = b.dataset.f;
    for (const x of $("filters").children) x.classList.toggle("on", x === b);
    renderList();
  });
  $("pg-prev").addEventListener("click", () => { st.page = Math.max(0, st.page - 1); renderScreen(); });
  $("pg-next").addEventListener("click", () => { st.page = Math.min(screenPages() - 1, st.page + 1); renderScreen(); });
  $("emu-keys").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) emuKey(b.dataset.k); });
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
    if (e.target.closest("input, textarea, summary")) return;
    // Enter and Space on a button press it, the emulator's on-screen keys included
    if ((e.key === "Enter" || e.key === " ") && e.target.closest("button, [role=button]")) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (emuMode()) {
      const k = KEYMAP[e.key];
      if (k) { e.preventDefault(); emuKey(k); }
      return;
    }
    if (e.key === "ArrowRight") $("pg-next").click();
    if (e.key === "ArrowLeft") $("pg-prev").click();
  });
  if ("serial" in navigator) {
    navigator.serial.addEventListener("disconnect", (e) => {
      if (e.target === st.port) { st.port = null; if (!emuMode()) connState("off", "Unplugged"); log("sys", "unplugged"); updateButtons(); }
    });
  }
}
main();
