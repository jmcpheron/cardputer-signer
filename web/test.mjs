// The page's own hash, blockies and passkey checks against the vectors, and the emulator if
// web/build.sh has built it: node web/test.mjs
import { existsSync, readFileSync } from "node:fs";
import { load as loadEmu } from "./lib/emu.js";
import { build } from "./lib/blockies.js";
import { fields, hex, keccak256, safeTxHash, tamper, txLine } from "./lib/safe.js";
import { b64url, parseAttestation, parseDer, safeSignature, verify } from "./lib/passkey.js";

const load = (n) => JSON.parse(readFileSync(new URL(`../vectors/${n}`, import.meta.url)));
let failed = 0;
const check = (ok, what) => { if (!ok) { failed++; console.log("FAIL", what); } };

check(hex(keccak256(new Uint8Array())) === "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470", "keccak256('')");
// 136 bytes: exactly one block of input, so the padding takes a block of its own
check(hex(keccak256(new Uint8Array(136))).length === 64, "keccak256 of a full block");

const vectors = load("safe_tx.json").vectors;
for (const v of vectors) {
  const h = safeTxHash(fields(v));
  check(h.domain === v.domainSeparator.toLowerCase(), `${v.name}: domain separator`);
  check(h.hash === v.safeTxHash.toLowerCase(), `${v.name}: safeTxHash`);
}
const line = txLine(fields(vectors[0]), tamper(vectors[0].safeTxHash));
check(line.startsWith("tx chain=") && !line.endsWith(vectors[0].safeTxHash), "txLine with a tampered expect");

const blockies = load("blockies.json").vectors;
for (const v of blockies) {
  const b = build(v.seed);
  check(b.cells.join("") === v.cells, `${v.seed}: cells`);
  for (const k of ["bg", "color", "spot"]) check(b[k].join() === v[k].join(), `${v.seed}: ${k}`);
}

// The passkey side, with Node as the authenticator: a P-256 key, and the bytes a browser makes.
const N = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551n;
const cat = (...p) => Uint8Array.from(p.flatMap((x) => [...x]));
const sha = async (b) => new Uint8Array(await crypto.subtle.digest("SHA-256", b));
const big = (b) => BigInt("0x" + hex(b));
const der = (r, s) => {
  const int = (n) => { let b = Buffer.from(n.toString(16).padStart(64, "0"), "hex"); while (b.length > 1 && b[0] === 0 && !(b[1] & 0x80)) b = b.subarray(1); if (b[0] & 0x80) b = Buffer.concat([Buffer.of(0), b]); return [2, b.length, ...b]; };
  const body = [...int(r), ...int(s)];
  return Uint8Array.from([0x30, body.length, ...body]);
};
const kp = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
const raw = new Uint8Array(await crypto.subtle.exportKey("raw", kp.publicKey));
const x = raw.slice(1, 33), y = raw.slice(33);
const rpId = "diykey.app", origin = "https://diykey.app";
const hash = safeTxHash(fields(vectors[1])).hash;
const authData = cat(await sha(new TextEncoder().encode(rpId)), [0x05], [0, 0, 0, 7]);
async function assertion(h, flip) {
  const client = new TextEncoder().encode(JSON.stringify({ type: "webauthn.get", challenge: b64url(Buffer.from(h.slice(2), "hex")), origin, crossOrigin: false }));
  const p1363 = new Uint8Array(await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, kp.privateKey, cat(authData, await sha(client))));
  let r = big(p1363.slice(0, 32)), s = big(p1363.slice(32));
  if (flip) s = N - s; // the other, high-s form of the same signature
  return { authenticatorData: authData, clientDataJSON: client, signature: der(r, s), x, y, hash, rpId, origin };
}
for (const flip of [false, true]) {
  const v = await verify(await assertion(hash, flip));
  check(v.ok, `passkey: a good assertion verifies${flip ? " (high s)" : ""}: ${v.checks.filter(([, ok]) => !ok).map(([k]) => k)}`);
  check(BigInt(v.s) <= N / 2n, "passkey: s comes back low");
  check(v.counter === 7 && v.clientDataFields === `"origin":"${origin}","crossOrigin":false`, "passkey: counter and clientDataFields");
}
const other = await verify({ ...(await assertion(tamper(hash))), hash });
check(!other.ok && !other.checks[0][1], "passkey: a signature over another hash fails the challenge check");
check(!(await verify({ ...(await assertion(hash)), rpId: "evil.example" })).ok, "passkey: another site's assertion fails");
// The Safe encoding: four head words, then authenticatorData and clientDataFields, each padded.
const enc = safeSignature(authData, "ab", 1n, 2n).slice(2);
const w = (i) => BigInt("0x" + enc.slice(i * 64, i * 64 + 64));
check(w(0) === 128n && w(1) === 128n + 32n + 64n && w(2) === 1n && w(3) === 2n && w(4) === 37n && w(7) === 2n && enc.length === 9 * 64, "passkey: Safe signature encoding");
// A registration: CBOR attestation object with a COSE key, made by hand.
const cb = (v) => {
  const head = (m, n) => (n < 24 ? [m << 5 | n] : n < 256 ? [m << 5 | 24, n] : [m << 5 | 25, n >> 8, n & 255]);
  if (typeof v === "number") return v >= 0 ? head(0, v) : head(1, -1 - v);
  if (typeof v === "string") { const b = [...new TextEncoder().encode(v)]; return [...head(3, b.length), ...b]; }
  if (v instanceof Uint8Array) return [...head(2, v.length), ...v];
  return [...head(5, v.length), ...v.flatMap(([k, val]) => [...cb(k), ...cb(val)])];
};
const credId = Uint8Array.from({ length: 20 }, (_, i) => i);
const cose = Uint8Array.from(cb([[1, 2], [3, -7], [-1, 1], [-2, x], [-3, y]]));
const regAuth = cat(await sha(new TextEncoder().encode(rpId)), [0x45], [0, 0, 0, 0], new Uint8Array(16), [0, 20], credId, cose);
const parsed = parseAttestation(Uint8Array.from(cb([["fmt", "none"], ["attStmt", []], ["authData", regAuth]])));
check(hex(parsed.id) === hex(credId) && hex(parsed.x) === hex(x) && hex(parsed.y) === hex(y), "passkey: registration parsed");
check(parseDer(der(5n, N - 1n)).s === N - 1n, "passkey: DER");

// The emulator: the signer's core in WebAssembly, through its states.
const wasm = new URL("emu/signer.wasm", import.meta.url);
let emuNote = "emulator skipped (run web/build.sh)";
if (existsSync(wasm)) {
  const emu = await loadEmu(readFileSync(wasm));
  emu.boot();
  for (const v of vectors) {
    const got = emu.send(txLine(fields(v), v.safeTxHash.toLowerCase()));
    check(got[0] === "= hash " + v.safeTxHash.toLowerCase() && !got.includes("= mismatch"), `emulator: ${v.name} hash`);
    check(emu.key("del")[0] === "= rejected" && emu.key("enter").length === 0, `emulator: ${v.name} rejected`);
  }
  const v = vectors[1], h = v.safeTxHash.toLowerCase();
  const toHash = () => { for (let i = 0; i < 6; i++) emu.key("right"); };
  emu.send(txLine(fields(v), h)); toHash();
  check(emu.key("enter").length === 0, "emulator: no key, no signing");
  emu.setPasskey("0123456789abcdef");
  check(emu.key("enter")[0] === "= sign " + h, "emulator: asks to sign its own hash");
  check(emu.send(txLine(fields(v), h))[0] === "= busy", "emulator: busy while signing");
  check(emu.signResult(true)[0] === "= signed" && emu.key("enter").length === 0, "emulator: signed, then idle");
  emu.send(txLine(fields(v), tamper(h))); toHash();
  check(emu.key("enter").length === 0 && emu.key("del")[0] === "= rejected", "emulator: a wrong claim can't be signed");
  emu.key("enter");
  check(emu.send("sign " + h)[0] === "= error unknown command", "emulator: no sign command over the line");
  emuNote = `emulator (${vectors.length} hashes, signing rules)`;
}

console.log(failed ? `${failed} failed` : `ok: keccak, ${vectors.length} Safe hashes, ${blockies.length} blockies, passkey signatures, ${emuNote}`);
process.exit(failed ? 1 : 0);
