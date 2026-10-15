// The page: demo transactions, sent to a signer, and the firmware flashed with esptool-js. The
// signer is either the emulator (the signer's core in WebAssembly, with a passkey as its key) or a
// Cardputer over Web Serial. Both answer in the same lines (core/src/proto.rs), so one handler
// reads both. Everything the page loads is in this folder or the repository's vectors/.
import { canvas as blockie } from "./lib/blockies.js";
import * as ceremony from "./lib/ceremony.js";
import { load as loadEmu } from "./lib/emu.js";
import * as passkey from "./lib/passkey.js";
import { fields, safeTxHash, tamper as tampered, txLine } from "./lib/safe.js";
import { fingerprint, signerAddress } from "./lib/signer.js";

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
  // what the signer has said about itself: { fw, chip: { model, serial, locks, counter }, pubkey, missing }
  dev: {},
  pending: null, // { name, claimed, device, pages, done, signed }
  firmware: null, fwPick: "signer", flashing: false,
  cer: null, // the key ceremony's lines, read and checked (lib/ceremony.js), once its image talks
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
  emu: "The Cardputer below runs the real firmware's code, in this page, with your passkey as its key. Send it a demo Safe transaction: it works out the hash itself, shows you every page, and signs only that hash. Claim a wrong hash, and it turns red and won't sign.",
  usb: "Plug in your Cardputer and send it a demo Safe transaction. It works out the hash itself, shows you every page on its own screen, and only compares the hash this page claims. A wrong claim turns it red.",
};

function setMode(m) {
  st.mode = m;
  const emu = emuMode();
  for (const b of $("modes").children) b.classList.toggle("on", b.dataset.m === m);
  document.body.classList.toggle("is-emu", emu);
  $("lede").textContent = LEDE[m];
  const labels = emu
    ? { "lab-wire": "WebAuthn",
        "mini-key": "Passkey", "mini-wire": "WebAuthn", "pc-title": "This page · the emulator runs here" }
    : { "lab-wire": "Grove",
        "mini-key": "ATECC608", "mini-wire": "Grove", "pc-title": "This computer · USB to the Cardputer" };
  for (const [id, text] of Object.entries(labels)) $(id).textContent = text;
  hint(emu);
  $("step4-title").textContent = emu ? "Your passkey signs" : "The chip signs";
  $("step4-sub").textContent = emu ? "Enter on the last page: the passkey signs the emulator's hash, and nothing else."
    : "Milestone 3: the chip signs that hash, and nothing else.";
  $("steps").lastElementChild.classList.toggle("future", !emu);
  $("send").textContent = emu ? "Send to the emulator" : "Send to the Cardputer";
  $("c-device-k").textContent = emu ? "The emulator worked out" : "The Cardputer worked out";
  $("connect").hidden = emu;
  $("emu-screen").hidden = !emu;
  $("d-screen").hidden = emu;
  $("sig").hidden = emu;
  st.dev = {};
  st.kpOpen = null;
  st.found = null;
  st.pending = null;
  st.usbScreen = "idle";
  $("u-line").replaceChildren(el("span", "none", "nothing yet"));
  if (emu) {
    connState("on", "Emulator");
    if (st.emu) emuLines(st.emu.boot());
    renderPasskey();
    drawEmu();
    requestAnimationFrame(routeCable);
  } else {
    connState(st.port ? "on" : "off", st.port ? "Connected" : "Not connected");
    if (st.port) send("ping");
  }
  renderKeycard();
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
/** In USB mode the drawn Cardputer follows the real one: idle, its pages once it has answered (the
 * keys here page through the same pictures), then rejected. In the emulator, the emulator draws. */
function renderScreen() {
  if (emuMode()) return drawEmu();
  let name = "idle";
  if (st.usbScreen === "rejected") name = "rejected";
  else if (st.usbScreen === "review" && st.sel) {
    const n = screenPages();
    name = st.page === n - 1 && st.tamper ? `${st.sel.name}-tampered-${n}` : `${st.sel.name}-${st.page + 1}`;
  }
  $("d-screen").src = `screens/${name}.png`;
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
  requestAnimationFrame(routeCable);
}

// ------------------------------------------------------------------ the emulator
function drawEmu() {
  if (!st.emu) return;
  $("emu-screen").getContext("2d").putImageData(new ImageData(st.emu.pixels(), st.emu.w, st.emu.h), 0, 0);
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

/** A key on the drawn Cardputer: the emulator's, or in USB mode, paging the pictures along with the
 * real device (its own keys do the real paging). */
function press(name) {
  flashCap(name);
  if (emuMode()) return emuKey(name);
  if (st.usbScreen !== "review") return;
  const n = screenPages();
  if ((name === "right" || name === "down" || name === "enter") && st.page < n - 1) st.page++;
  else if ((name === "left" || name === "up") && st.page > 0) st.page--;
  renderScreen();
}

// ------------------------------------------------------------------ the cables
/** Draws the key's wires into the Grove port, and the cable from the Cardputer's USB-C port into
 * this computer's port, wherever the layout has put them. Hidden when the bench is one column. */
function routeCable() {
  const svg = $("usb-overlay");
  if (getComputedStyle(svg).display === "none") return;
  // the port sits between what goes down the cable and what comes back up it
  const pc = $("pc").getBoundingClientRect(), down = $("d-line").getBoundingClientRect(), up = $("u-line").getBoundingClientRect();
  $("pc-port").style.top = `${Math.round((down.top + up.bottom) / 2 - pc.top - 12)}px`;
  const b = $("bench").getBoundingClientRect(), u = document.querySelector(".cp-usb").getBoundingClientRect(), q = $("pc-port").getBoundingClientRect();
  const x1 = u.right - b.left, y1 = u.top + u.height / 2 - b.top, x2 = q.left - b.left, y2 = q.top + q.height / 2 - b.top;
  // out of the port, a loop down, and in from the left: a cable hanging between them
  const dx = Math.max(40, (x2 - x1) * 0.9);
  const d = `M${x1 + 18} ${y1} C ${x1 + 18 + dx} ${y1 + 10}, ${x2 - dx} ${y2 + 30}, ${x2} ${y2}`;
  $("usb-cable").setAttribute("d", d);
  $("pulse-tx").setAttribute("d", d);
  $("pulse-rx").setAttribute("d", d);
  const plug = $("usb-plug");
  plug.setAttribute("x", x1 - 6);
  plug.setAttribute("y", y1 - 8);

  // the key: out of its panel's port and into the Grove port, one wire per pin
  const k = $("kp-port").getBoundingClientRect(), g = document.querySelector(".cp-grove").getBoundingClientRect();
  const kx = k.right - b.left, ky = k.top + k.height / 2 - b.top, gx = g.left - b.left + 2, gy = g.top + g.height / 2 - b.top;
  const kdx = Math.max(24, (gx - kx) * 0.55);
  const wire = (o, p) => `M${kx} ${ky + o} C ${kx + kdx} ${ky + o}, ${gx - kdx} ${gy + p}, ${gx} ${gy + p}`;
  const step = g.height / 5;
  document.querySelectorAll("#usb-overlay .w").forEach((w, i) => w.setAttribute("d", wire((i - 1.5) * 4, (i - 1.5) * step)));
  $("webauthn").setAttribute("d", wire(0, 0));
  $("pulse-key").setAttribute("d", wire(0, 0));
  const lab = $("lab-wire");
  lab.setAttribute("x", (kx + gx) / 2);
  lab.setAttribute("y", Math.min(ky, gy) - 12);
}

// ------------------------------------------------------------------ the drawn Cardputer's keyboard
const ROWS = [
  [["`", "esc"], ["1", "!"], ["2", "@"], ["3", "#"], ["4", "$"], ["5", "%"], ["6", "^"], ["7", "&"], ["8", "*"], ["9", "("], ["0", ")"], ["-", "_"], ["=", "+"], ["del"]],
  [["tab"], ["q"], ["w"], ["e"], ["r"], ["t"], ["y"], ["u"], ["i"], ["o"], ["p"], ["[", "{"], ["]", "}"], ["\\", "|"]],
  [["fn"], ["Aa"], ["a"], ["s"], ["d"], ["f"], ["g"], ["h"], ["j"], ["k"], ["l"], [";"], ["'", "\""], ["ok"]],
  [["ctrl"], ["opt"], ["alt"], ["z"], ["x"], ["c"], ["v"], ["b"], ["n"], ["m"], [","], ["."], ["/"], ["space"]],
];
// The signer's keys, as the firmware reads them: ; . , / are the arrows printed on the real one.
const LIVE = { ";": "up", ".": "down", ",": "left", "/": "right", ok: "enter", del: "del" };
const ARROW = { ";": "▲", ".": "▼", ",": "◀", "/": "▶" };
const TAG = { esc: "t-esc", fn: "t-red", Aa: "t-blue", opt: "t-teal", ok: "t-ink", del: "t-ink" };
const SHOWN = { ok: "ok ↵", del: "⌫", esc: "esc" };

function buildKeyboard() {
  const kb = $("cp-keys");
  for (const row of ROWS) {
    for (const [main, alt] of row) {
      const live = LIVE[main];
      const k = el("div", "kb" + (live ? " live" : ""));
      const lab = el("span", "k-lab");
      if (main === "`") lab.append(el("span", "t t-esc", "esc"), el("small", "", "`"));
      else if (TAG[main]) lab.append(el("span", "t " + TAG[main], SHOWN[main] || main));
      else if (ARROW[main]) lab.append(el("span", "", main), el("span", "arr", ARROW[main]));
      else {
        lab.append(el("span", "", main.length === 1 ? main.toUpperCase() : main));
        if (alt) lab.append(el("small", "", alt));
      }
      const cap = el("button", "cap");
      cap.type = "button";
      cap.dataset.char = main === "ok" ? "enter" : main === "del" ? "backspace" : main === "space" ? " " : main.toLowerCase();
      if (live) {
        cap.dataset.live = live;
        cap.setAttribute("aria-label", { up: "; (up)", down: ". (down)", left: ", (previous page)", right: "/ (next page)", enter: "ok (Enter)", del: "del (reject)" }[live]);
        cap.addEventListener("click", () => press(live));
      } else {
        cap.tabIndex = -1;
        cap.setAttribute("aria-hidden", "true");
      }
      k.append(lab, cap);
      kb.append(k);
    }
  }
}

/** Shows a key going down on the drawn keyboard. */
function flashCap(name, char) {
  const cap = $("cp-keys").querySelector(name ? `[data-live="${name}"]` : `[data-char="${CSS.escape(char)}"]`);
  if (!cap) return;
  cap.classList.add("down");
  setTimeout(() => cap.classList.remove("down"), 120);
}

function hint(emu) {
  const h = $("stage-hint");
  const kbd = (t) => el("kbd", "", t);
  h.replaceChildren(...(emu
    ? [document.createTextNode("Press the Cardputer's keys, or your own: "), kbd(","), kbd("/"), document.createTextNode(" page, "),
       kbd("Enter"), document.createTextNode(" ok, "), kbd("Del"), document.createTextNode(" rejects. The screen is the firmware's own code, running in this page.")]
    : [document.createTextNode("This screen follows your Cardputer. Page through it on the real one: "), kbd(","), kbd("/"),
       document.createTextNode(", "), kbd("ok"), document.createTextNode(", "), kbd("del"), document.createTextNode(". The keys here page the pictures along with it.")]));
}

// ------------------------------------------------------------------ the key that's attached
/** The key panel, out to the left of the key: a tab with the signing account's blockie and short
 * address, which opens to the whole of it, and to making or loading a passkey. It starts open
 * while there's no key, and closes on its own once there is one. */
function renderKeycard() {
  const panel = $("keypanel"), tab = $("kp-tab"), body = $("kp-body");
  const emu = emuMode(), d = st.dev;
  // the public key: the passkey's own, or the one the chip reported for slot 0
  const pub = emu ? (st.key ? { x: st.key.x, y: st.key.y } : null) : d.pubkey ? { x: d.pubkey.slice(0, 64), y: d.pubkey.slice(64) } : null;
  const addr = pub ? signerAddress(pub.x, pub.y) : null;
  const open = st.kpOpen ?? !addr;
  panel.className = "keypanel " + (emu ? "kp-pk" : "kp-chip") + (open ? " open" : "");
  tab.setAttribute("aria-expanded", String(open));
  tab.title = open ? "Fold the key panel away" : "Open the key panel";

  // the tab: who signs, at a glance
  const txt = el("div", "kp-txt");
  txt.append(el("span", "kp-k", emu ? "Passkey" : "Chip"));
  if (addr) {
    txt.append(el("span", "kp-t1", emu ? `${st.key.id.slice(0, 4)} ${st.key.id.slice(4, 8)}` : d.chip.model),
      el("span", "kp-t2", `${addr.slice(0, 6)}…${addr.slice(-4)}`));
  } else {
    txt.append(el("span", "kp-t1", emu ? (st.found ? "Confirm it" : "Set one up") : d.chip ? d.chip.model : d.missing ? "No chip" : st.port ? "Not read yet" : "Not connected"));
  }
  tab.replaceChildren(addr ? blockie(addr.toLowerCase(), 48) : el("div", "kp-ico", emu ? "🔑" : "▦"), txt,
    el("span", "kp-chev", open ? "◂" : "▸"));

  // the body
  body.replaceChildren();
  if (addr) {
    const box = el("div", "kc-addr");
    box.append(el("span", "kc-k", "Signs on chain as"));
    const row = el("div", "kc-addr-row");
    const copy = el("button", "btn btn-ghost", "Copy");
    copy.addEventListener("click", () => navigator.clipboard?.writeText(addr).then(() => { copy.textContent = "Copied"; setTimeout(() => (copy.textContent = "Copy"), 1200); }));
    row.append(el("code", "", addr), copy);
    const fp = el("span", "fine", "P-256 key");
    fingerprint(pub.x, pub.y).then((f) => (fp.textContent = `P-256 key, fingerprint ${f}`));
    box.append(row, fp);
    body.append(box);
  } else if (emu && st.found) {
    body.append(el("p", "kp-step", `Found passkey ${toHexShort(st.found.id)}. Confirm it once more: from two signatures this page works out its public key, which a passkey only hands out when it's made.`));
  } else if (emu) {
    body.append(el("p", "", "Your phone or computer makes a P-256 key inside its authenticator (Face ID, Touch ID, Windows Hello, a security key). It stands in for the ATECC608."));
  } else if (d.chip && !d.pubkeyRead) {
    body.append(el("p", "", "This firmware doesn't report the chip's public key. Flash the latest firmware below, then press Enter on the Cardputer's idle screen."));
  } else if (d.chip) {
    body.append(el("p", "", "Slot 0 has no key yet, so there is no address. Making a key in the chip is a permanent step, and this firmware never does it."));
  } else if (st.port) {
    body.append(el("p", "", "Press Enter on the Cardputer's idle screen: it reads the chip and reports it here."));
  } else {
    body.append(el("p", "", "Connect your Cardputer over USB. It reads the chip on its Grove port and reports its key here."));
  }

  const tags = el("div", "kc-tags");
  const tag = (k, v) => { const t = el("span", "tag"); t.append(document.createTextNode(k + " "), el("b", "", v)); tags.append(t); };
  if (d.fw) tag("firmware", d.fw);
  if (emu && st.key) { tag("export", "can't"); if (st.lastSig) tag("sign count", String(st.lastSig.counter)); }
  if (!emu && d.chip) { tag("serial", d.chip.serial.slice(0, 8) + "…"); tag("config", d.chip.config); tag("data", d.chip.data); tag("counter 0", d.chip.counter); }
  if (tags.childNodes.length) body.append(tags);

  const actions = el("div", "kc-actions");
  const btn = (cls, text, fn) => { const b = el("button", "btn " + cls, text); b.addEventListener("click", fn); actions.append(b); };
  if (emu && st.found) {
    btn("btn-primary", "Confirm", confirmPasskey);
    btn("btn-ghost", "Cancel", () => { st.found = null; renderKeycard(); });
  } else if (emu && !st.key) {
    btn("btn-primary", "Make a passkey", makePasskey);
    btn("", "Use one I made before", findPasskey);
    if (!window.PublicKeyCredential) actions.append(el("span", "pk-note bad", "This browser has no passkeys (WebAuthn)."));
  } else if (emu) {
    btn("btn-ghost", "Forget this passkey", () => {
      passkey.forget(); st.key = null; st.lastSig = null; st.kpOpen = null;
      if (st.emu) emuLines(st.emu.setPasskey(null));
      renderPasskey(); renderKeycard();
    });
  }
  if (actions.childNodes.length) body.append(actions);

  if (addr) {
    const note = el("details", "kc-note");
    note.append(el("summary", "", "What is this address?"),
      el("p", "", "A P-256 key can't be an Ethereum account by itself. Safe gives each one its own small signer contract, at this address, the same on every chain Safe's passkey factory is on (Ethereum, Base, and others). It only checks this key's signatures for a Safe."),
      el("p", "", "It isn't a wallet: don't send ETH or tokens to it, because it has no way to move them. To hold funds, make a Safe with this address as one of its owners, and send them to the Safe. That's a good use for a diykey, though setting one up is outside this page."));
    body.append(note);
  }
  requestAnimationFrame(routeCable);
}

const toHexShort = (b) => Array.from(b.slice(0, 4), (x) => x.toString(16).padStart(2, "0")).join("").replace(/(.{4})/, "$1 ");

/** A key is set: fold the panel away. */
function keyIsSet() {
  st.kpOpen = false;
  if (st.emu) emuLines(st.emu.setPasskey(st.key.id));
  renderKeycard();
}

async function findPasskey() {
  try {
    st.found = await passkey.find();
    log("sys", "passkey found; confirm it to work out its public key");
  } catch (e) {
    log("sys", `passkey: ${e.name}: ${e.message}`);
    status(e.name === "NotAllowedError" ? "No passkey picked (cancelled, or none made on this site)." : `Couldn't find a passkey: ${e.message}`, "bad");
  }
  renderKeycard();
}

async function confirmPasskey() {
  try {
    st.key = await passkey.confirm(st.found);
    log("sys", `passkey loaded: ${st.key.id.slice(0, 16)}…, its public key worked out from two signatures`);
    st.found = null;
    keyIsSet();
  } catch (e) {
    log("sys", `passkey: ${e.name}: ${e.message}`);
    status(e.name === "NotAllowedError" ? "Not confirmed (cancelled)." : `Couldn't load that passkey: ${e.message}`, "bad");
    renderKeycard();
  }
}

/** The passkey's last signature, in detail, under the comparison. */
function renderPasskey() {
  const pk = $("pk");
  pk.replaceChildren();
  pk.hidden = !emuMode() || !st.lastSig;
  if (st.lastSig) pk.append(sigAnatomy(st.lastSig));
}

async function makePasskey() {
  try {
    st.key = await passkey.create();
    log("sys", `passkey made: ${st.key.id.slice(0, 16)}…`);
    keyIsSet();
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
  pulse("chip", 0);
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
      renderKeycard();
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
const pulses = {};
/** Data along a cable for a moment: "tx" down the USB cable, "rx" back up it, "chip" over the key's
 * wires. With `ms` 0 it runs until stopPulse. */
function pulse(kind, ms = 900) {
  const f = $("bench");
  clearTimeout(pulses[kind]);
  f.classList.add("p-" + kind);
  if (ms) pulses[kind] = setTimeout(() => f.classList.remove("p-" + kind), ms);
}
function stopPulse(kind) {
  clearTimeout(pulses[kind]);
  $("bench").classList.remove("p-" + kind);
}
function resetSend() {
  steps({});
  stopPulse("chip");
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
  if (!emuMode()) { st.usbScreen = "idle"; renderScreen(); }
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
  stopPulse("chip");
  const who = emuMode() ? "The emulator" : "The Cardputer";
  if (same) {
    status(`${who} worked out the same hash. Page through it ${emuMode() ? "with › or /" : "on the device"}: ${p.pages || "its"} pages. ${emuMode() && st.key ? "Enter on the last page signs. " : ""}Del rejects.`, "good");
  } else if (honest) {
    status(`${who} worked out the real hash and saw this page's claim is wrong. Its hash page is red: it says don't sign, and it won't. Del rejects.`, "bad");
  } else {
    status(`${who}'s hash is different from the one this browser worked out. Don't trust either until you know why.`, "bad");
  }
  if (!emuMode()) { st.usbScreen = "review"; st.page = 0; renderScreen(); }
}

// ------------------------------------------------------------------ lines from the signer
/** The last few lines that came back up the cable. */
function upLine(line, reply, bad) {
  const pre = $("u-line");
  pre.querySelector(".none")?.remove();
  pre.append(el("div", reply ? (bad ? "r bad" : "r") : "", line));
  while (pre.childNodes.length > 4) pre.firstChild.remove();
  pre.scrollTop = pre.scrollHeight;
}
function onLine(line) {
  line = line.replace(/\r$/, "");
  if (!line.trim()) return;
  const reply = line.startsWith("= ");
  const bad = /^= (error|mismatch|busy|not signed)/.test(line);
  log(reply ? (bad ? "in bad" : "in") : "dev", line);
  upLine(line, reply, bad);
  pulse("rx");

  let m;
  const d = st.dev;
  if (!emuMode()) ceremonyLine(line);
  if ((m = line.match(/^cardputer-signer: (.+)$/)) || (m = line.match(/^= pong (cardputer-signer .+)$/))) {
    const what = m[1].replace(/^cardputer-signer /, "");
    d.fw = what === "emulator" ? "emulator (WebAssembly)" : what.replace(/^m(\d)$/, "milestone $1");
    if (!emuMode()) connState("on", "Connected");
    renderKeycard();
  }
  if ((m = line.match(/^chip: serial \[([0-9a-f, ]+)\] revision \[([0-9a-f, ]+)\] config locked (\w+) data locked (\w+) counter0 (\d+)/))) {
    const serial = m[1].replace(/[ ,]/g, ""), rev = m[2].replace(/[ ,]/g, "");
    const model = rev.endsWith("6002") ? "ATECC608A" : rev.endsWith("6003") ? "ATECC608B" : "ATECC608";
    d.chip = { model, serial, config: m[3] === "true" ? "locked" : "open", data: m[4] === "true" ? "locked" : "open", counter: m[5] };
    d.missing = false;
    d.pubkey = null;
    d.pubkeyRead = false;
    pulse("chip", 1400);
    renderKeycard();
  } else if ((m = line.match(/^chip: pubkey0 (0x[0-9a-f]{128}|none)$/))) {
    d.pubkeyRead = true;
    d.pubkey = m[1] === "none" ? null : m[1].slice(2);
    renderKeycard();
  } else if (line.startsWith("chip: passkey ") || line.startsWith("chip: no key")) {
    pulse("chip", 1400);
    renderKeycard();
  } else if (line.startsWith("chip: no answer")) {
    d.chip = null;
    d.missing = true;
    renderKeycard();
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
    stopPulse("chip");
    status("Signed by your passkey over the emulator's hash, and the signature checked out in this page. Enter on the emulator goes back.", "good");
    updateButtons();
  } else if (line === "= not signed") {
    steps({ 1: "done", 2: "done", 3: "active" });
    stopPulse("chip");
  } else if (line === "= rejected") {
    p.done = true;
    steps({ 1: "done", 2: p.device === st.hash.hash ? "done" : "fail", 3: "done" });
    status(`Rejected on ${signerName()}. Nothing was signed; the key wasn't used.`, "");
    if (!emuMode()) { st.usbScreen = "rejected"; renderScreen(); }
    updateButtons();
  } else if (line === "= busy") {
    clearTimeout(p.timer);
    p.done = true;
    steps({ 1: "fail" });
    stopPulse("chip");
    status(`A transaction is already on ${signerName()}'s screen. Press Del there to reject it, then send again.`, "bad");
    updateButtons();
  } else if ((m = line.match(/^= error (.*)$/))) {
    clearTimeout(p.timer);
    p.done = true;
    steps({ 1: "fail" });
    stopPulse("chip");
    status(`${emuMode() ? "The emulator" : "The Cardputer"} refused it: ${m[1]}.`, "bad");
    updateButtons();
  }
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
  $("flash-btn").disabled = !fwImage() || !$("fw-ok").checked || st.flashing || !("serial" in navigator);
}

async function openPort(port) {
  await port.open({ baudRate: 115200 });
  st.port = port;
  st.lineBuf = "";
  log("sys", "port open");
  st.reading = readLoop(port);
  if (!emuMode()) {
    connState("on", "Connected");
    // it prints the chip only at start-up, or on Enter at its idle screen; opening the port doesn't reset it
    renderKeycard();
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
    pulse("tx");
    // after the caller has set up, as a Cardputer's reply comes after the write
    queueMicrotask(() => emuLines(st.emu.send(line)));
    return;
  }
  // one write at a time: a reply can come back, and ask for the next line, before a write is done
  const write = async () => {
    if (!st.port?.writable) return;
    const w = st.port.writable.getWriter();
    try {
      await w.write(new TextEncoder().encode(line + "\n"));
      log("out", shown);
      pulse("tx");
    } finally {
      w.releaseLock();
    }
  };
  st.writing = (st.writing || Promise.resolve()).catch(() => {}).then(write);
  return st.writing;
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
  status("Sending the fields…");
  const phone = matchMedia("(max-width: 760px)").matches;
  if (emuMode()) $("emu-screen").focus({ preventScroll: phone }); // so Enter and the page keys go to the emulator
  await send(txLine(fields(v), p.claimed));
  // on a phone this computer sits above the Cardputer: bring it up, once the lines above it have grown
  if (phone) $("stage").scrollIntoView({ block: "start" });
  if (p.done || p.device) return; // the answer came back before the write finished
  steps({ 1: "done", 2: "active" });
  status(`Waiting for ${signerName()}'s hash…`);
  p.timer = setTimeout(() => {
    if (p.device || p.done) return;
    p.done = true;
    steps({ 1: "done", 2: "fail" });
    stopPulse("chip");
    status("No answer in 4 seconds. Is the signer firmware on the board? The log below shows what came back.", "bad");
    updateButtons();
  }, 4000);
}

// ------------------------------------------------------------------ the key ceremony
/** A line from the Cardputer while the ceremony image runs: read and checked by lib/ceremony.js. */
function ceremonyLine(line) {
  if (line === "= pong diykey-ceremony") {
    // it may have printed its first steps before this page was listening: ask for all of them again
    st.cer = ceremony.reader();
    st.dev.fw = "key ceremony";
    renderCeremony();
    send("log");
    return;
  }
  if (line.startsWith("diykey-ceremony: ") && !st.cer) st.cer = ceremony.reader();
  if (!st.cer) return;
  st.cer.feed(line).then(renderCeremony);
}

const STEPS = { p1: "P1 Read the chip", p2: "P2 Write the rules", p3: "P3 Lock the rules", p4: "P4 Zeros and the record",
  p5: "P5 Lock the data", p6: "P6 Check", k1: "K1 Make your key", k2: "K2 Test it", k3: "K3 Seal it", k4: "K4 Its address" };

function renderCeremony() {
  const c = st.cer;
  const appears = c && $("cer").hidden;
  $("cer").hidden = !c;
  if (!c) return;
  if (appears) $("cer").scrollIntoView({ block: "start", behavior: "smooth" });
  const last = c.lines.filter((l) => /^(p\d|k\d): /.test(l)).at(-1);
  const step = last ? last.slice(0, 2) : null;
  $("cer-title").textContent = c.stop ? "Stopped" : step === "k4" ? "Done: a sealed key" : step ? `Last: ${STEPS[step]}` : "Connected to the ceremony image";
  $("cer-title").className = c.stop ? "stop" : "";
  $("cer-sub").textContent = c.stop ? c.stop + " Del on the Cardputer starts again from where the chip is."
    : step === "k4" ? "Save the log. Then flash the signer back: The signer, under Flash a Cardputer below."
    : step === "k2" ? "Seal the key on the Cardputer only if both test signatures are valid below."
    : "Do each step on the Cardputer. This page checks what it prints, on its own.";

  const dl = $("cer-vals");
  dl.replaceChildren();
  const row = (k, v) => { dl.append(el("dt", "", k), el("dd", "", v)); };
  if (c.serial) row("Serial", c.serial.replace(/(.{4})/g, "$1 ").trim());
  if (c.commit) row("Image built from", c.commit);
  if (c.configCrc) row("Config CRC", `${c.configCrc} (this page: ${c.pageConfigCrc || "?"})`);
  if (c.dataCrc) row("Data CRC", c.dataCrc);
  if (c.pubkey) row("Public key", "0x" + c.pubkey);
  if (c.address) row("Safe signer address", c.address);

  const ul = $("cer-checks");
  ul.replaceChildren();
  for (const k of c.checks) {
    const li = el("li", k.ok === true ? "yes" : k.ok === false ? "no" : "unk");
    li.append(el("span", "step", k.step), document.createTextNode(k.what + " "), el("span", "why", k.detail ? `· ${k.detail}` : ""));
    ul.append(li);
  }
}

/** The log, as a text file: every line the ceremony printed, and what this page checked. Nothing
 * in it is secret. */
function saveCeremony() {
  const c = st.cer;
  if (!c) return;
  const f = st.flashed;
  const text = ["diykey key ceremony log", `saved ${new Date().toISOString()}`,
    ...(f ? [`flashed from this page: ${f.file}, sha256 ${f.sha256}, built from ${f.commit}`] : []),
    "", ...c.lines, "", "checked by the page:",
    ...c.checks.map((k) => `${k.ok === true ? "ok  " : k.ok === false ? "FAIL" : "?   "} ${k.step} ${k.what}${k.detail ? ": " + k.detail : ""}`), ""].join("\n");
  const a = el("a");
  a.href = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
  a.download = `diykey-ceremony-${c.serial || "chip"}.txt`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

// ------------------------------------------------------------------ flashing
/** The image picked to flash: the signer, or the key ceremony. */
const fwImage = () => st.firmware && (st.firmware.images ? st.firmware.images[st.fwPick] : st.fwPick === "signer" ? st.firmware : null);

async function loadFirmware() {
  try {
    st.firmware = await (await fetch("firmware/firmware.json", { cache: "no-store" })).json();
  } catch {
    st.firmware = null;
  }
  renderFirmware();
}

function renderFirmware() {
  const dl = $("fw-info"), info = st.firmware, img = fwImage();
  for (const b of $("fw-pick").children) b.classList.toggle("on", b.dataset.img === st.fwPick);
  $("flash-btn").textContent = st.fwPick === "ceremony" ? "Flash the key ceremony" : "Flash the signer";
  if (!img) {
    dl.replaceChildren(el("dt", "", "Image"), el("dd", "warn", "Not built yet: run web/build.sh, then reload."));
    updateButtons();
    return;
  }
  dl.replaceChildren();
  const row = (k, ...v) => { const dd = el("dd"); dd.append(...v); dl.append(el("dt", "", k), dd); };
  row("Image", el("code", "", img.file), document.createTextNode(` · ${img.size.toLocaleString()} bytes at 0x0`));
  const sha = el("code");
  sha.append(document.createTextNode(img.sha256.slice(0, 56)), el("b", "", img.sha256.slice(56)));
  row("SHA-256", sha);
  row("Built from", el("code", "", info.commit), document.createTextNode(` · ${info.built}`),
    ...(info.dirty ? [el("span", "warn", " · with uncommitted changes")] : []));
  row("For", document.createTextNode(`${info.chip.toUpperCase().replace(/^ESP32/, "ESP32-")}, ${info.flashSize} flash (Cardputer v1.1)`));
  if (st.fwPick === "ceremony") {
    row("Does", el("span", "warn", "Writes the chip's rules, locks them, makes its key and seals it, a step at a time, each one on the Cardputer's own screen. The locks and the seal can't be undone."));
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
    const img = fwImage();
    const data = new Uint8Array(await (await fetch(`firmware/${img.file}`, { cache: "no-store" })).arrayBuffer());
    const got = await sha256hex(data);
    if (got !== img.sha256) throw new Error(`the image's SHA-256 is ${got}, not the one in firmware.json`);
    log("sys", `image ok: ${img.file}, ${data.length} bytes, sha256 ${got}`);
    st.flashed = { file: img.file, sha256: got, commit: st.firmware.commit };

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
    stage(`Done. ${img.file === "diykey-ceremony.bin" ? "The key ceremony" : "The signer"} is on the board.`, 1);
    st.flashing = false;
    // the ceremony talks over USB: listen as the Cardputer, not the emulator
    if (img.file === "diykey-ceremony.bin" && emuMode()) setMode("usb");
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
  buildKeyboard();
  const ro = new ResizeObserver(routeCable);
  for (const id of ["bench", "keypanel", "cp"]) ro.observe($(id));
  addEventListener("load", routeCable);
  $("kp-tab").addEventListener("click", () => { st.kpOpen = !$("keypanel").classList.contains("open"); renderKeycard(); });
  select(st.vectors.find((v) => v.name === "eth-send-base-sepolia") || st.vectors[0]);
  // #usb opens on the Cardputer (USB), when this browser has Web Serial
  setMode(!st.emu || (location.hash === "#usb" && "serial" in navigator) ? "usb" : "emu");

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
  $("emu-keys").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) press(b.dataset.k); });
  $("tamper").addEventListener("change", (e) => {
    st.tamper = e.target.checked;
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
  $("fw-pick").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (b && !st.flashing) { st.fwPick = b.dataset.img; renderFirmware(); }
  });
  $("cer-save").addEventListener("click", saveCeremony);
  $("flash-btn").addEventListener("click", flash);
  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, textarea, summary")) return;
    // Enter and Space on a button press it, the emulator's on-screen keys included
    if ((e.key === "Enter" || e.key === " ") && e.target.closest("button, [role=button]")) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const k = KEYMAP[e.key];
    if (k) { e.preventDefault(); press(k); } else flashCap(null, e.key.toLowerCase());
  });
  if ("serial" in navigator) {
    navigator.serial.addEventListener("disconnect", (e) => {
      if (e.target === st.port) { st.port = null; if (!emuMode()) connState("off", "Unplugged"); log("sys", "unplugged"); updateButtons(); }
    });
  }
}
main();
