// A passkey as the emulator's key: a virtual ATECC608. Both use P-256, and the private key is made
// inside the authenticator (the Secure Enclave, a TPM, a security key) and can't be exported. A
// synced passkey (iCloud Keychain, Google Password Manager) is copied, encrypted, to the owner's
// other devices; the chip's key never leaves the chip.
//
// It signs in the WebAuthn format Safe's passkey signer checks: the challenge is the Safe
// transaction hash, and the signature is over authenticatorData ‖ sha256(clientDataJSON). The
// Cardputer will sign the same message at milestone 3, built on the device from its own hash.
// web/test.mjs checks the parsing, the verification and the encoding with a key made in Node.

const N = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551n; // P-256's order

export const b64url = (b) => btoa(String.fromCharCode(...b)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
export const fromB64url = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const fromHex = (h) => Uint8Array.from((h.replace(/^0x/, "").match(/../g) || []), (x) => parseInt(x, 16));
const concat = (...p) => { const o = new Uint8Array(p.reduce((n, x) => n + x.length, 0)); let i = 0; for (const x of p) { o.set(x, i); i += x.length; } return o; };
const sha256 = async (b) => new Uint8Array(await crypto.subtle.digest("SHA-256", b));
const big = (b) => BigInt("0x" + (hex(b) || "0"));
const word = (n) => fromHex(n.toString(16).padStart(64, "0"));

/** Just enough CBOR for an attestation object and a COSE key. Returns [value, next offset]. */
export function cbor(b, i = 0) {
  const ib = b[i++], major = ib >> 5, ai = ib & 31;
  let len = ai;
  if (ai === 24) len = b[i++];
  else if (ai === 25) { len = (b[i] << 8) | b[i + 1]; i += 2; }
  else if (ai === 26) { len = ((b[i] << 24) >>> 0) + ((b[i + 1] << 16) | (b[i + 2] << 8) | b[i + 3]); i += 4; }
  else if (ai > 26) throw new Error("CBOR: unsupported length");
  switch (major) {
    case 0: return [len, i];
    case 1: return [-1 - len, i];
    case 2: return [b.slice(i, i + len), i + len];
    case 3: return [new TextDecoder().decode(b.slice(i, i + len)), i + len];
    case 4: { const a = []; for (let k = 0; k < len; k++) { let v; [v, i] = cbor(b, i); a.push(v); } return [a, i]; }
    case 5: { const m = new Map(); for (let k = 0; k < len; k++) { let key, v; [key, i] = cbor(b, i); [v, i] = cbor(b, i); m.set(key, v); } return [m, i]; }
    default: throw new Error("CBOR: unsupported type " + major);
  }
}

/** The credential id and P-256 public key (x, y) out of a registration's attestationObject. */
export function parseAttestation(attestationObject) {
  const [att] = cbor(attestationObject);
  const auth = att.get("authData");
  if (!(auth[32] & 0x40)) throw new Error("no credential data in authenticatorData");
  const idLen = (auth[53] << 8) | auth[54];
  const id = auth.slice(55, 55 + idLen);
  const [key] = cbor(auth, 55 + idLen);
  if (key.get(1) !== 2 || key.get(3) !== -7 || key.get(-1) !== 1) throw new Error("not an ES256 key on P-256");
  return { id, x: key.get(-2), y: key.get(-3) };
}

/** A DER ECDSA signature as r and s. */
export function parseDer(der) {
  if (der[0] !== 0x30) throw new Error("not a DER signature");
  let i = 2;
  const int = () => { if (der[i] !== 0x02) throw new Error("bad DER"); const n = der[i + 1]; const v = der.slice(i + 2, i + 2 + n); i += 2 + n; return big(v); };
  return { r: int(), s: int() };
}

export async function importKey(x, y) {
  return crypto.subtle.importKey("raw", concat(Uint8Array.of(4), x, y), { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
}

/**
 * Checks an assertion against the hash it was meant to sign and the public key, as Safe's passkey
 * signer would: the challenge, the flags, then the P-256 signature over
 * authenticatorData ‖ sha256(clientDataJSON). Returns every part, for the page to show.
 */
export async function verify({ authenticatorData, clientDataJSON, signature, x, y, hash, rpId, origin }) {
  const client = new TextDecoder().decode(clientDataJSON);
  const want = b64url(fromHex(hash));
  const parsed = JSON.parse(client);
  const prefix = `{"type":"webauthn.get","challenge":"${want}",`;
  const checks = [
    ["the challenge is the hash", parsed.type === "webauthn.get" && parsed.challenge === want],
    ["for this site", parsed.origin === origin && hex(authenticatorData.slice(0, 32)) === hex(await sha256(new TextEncoder().encode(rpId)))],
    ["user present (UP)", !!(authenticatorData[32] & 0x01)],
    ["user verified (UV)", !!(authenticatorData[32] & 0x04)],
    ["clientDataJSON in the order Safe rebuilds", client.startsWith(prefix) && client.endsWith("}")],
  ];
  const message = concat(authenticatorData, await sha256(clientDataJSON));
  let { r, s } = parseDer(signature);
  if (s > N / 2n) s = N - s; // the low-s form: the same signature, in the form verifiers accept
  const key = await importKey(x, y);
  const ok = await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, key, concat(word(r), word(s)), message);
  checks.push(["P-256 signature valid", ok]);
  const fields = client.startsWith(prefix) ? client.slice(prefix.length, -1) : null;
  return {
    ok: checks.every(([, v]) => v), checks, client, challenge: want, flags: authenticatorData[32],
    counter: new DataView(authenticatorData.buffer, authenticatorData.byteOffset + 33, 4).getUint32(0),
    message: hex(message), r: "0x" + r.toString(16).padStart(64, "0"), s: "0x" + s.toString(16).padStart(64, "0"),
    clientDataFields: fields, safe: fields === null ? null : safeSignature(authenticatorData, fields, r, s),
  };
}

/** abi.encode(bytes authenticatorData, string clientDataFields, uint256 r, uint256 s): the
 * signature Safe's WebAuthn signer decodes. Not yet checked against a contract onchain. */
export function safeSignature(authenticatorData, clientDataFields, r, s) {
  const pad = (b) => concat(b, new Uint8Array((32 - (b.length % 32)) % 32));
  const a = authenticatorData, f = new TextEncoder().encode(clientDataFields);
  const offA = 128n, offF = 128n + 32n + BigInt(pad(a).length);
  return "0x" + hex(concat(word(offA), word(offF), word(r), word(s),
    word(BigInt(a.length)), pad(a), word(BigInt(f.length)), pad(f)));
}

// ------------------------------------------------------------------ the public key, from signatures
// WebAuthn hands out a passkey's public key only when the passkey is made. To use one made before
// (in another browser, or synced to a phone), the page works the key out from two signatures: one
// P-256 signature fits two public keys, and a second signature leaves the one that fits both. Plain
// curve arithmetic in BigInt; web/test.mjs checks it with keys made in Node.

const P = 0xffffffff00000001000000000000000000000000ffffffffffffffffffffffffn;
const A = P - 3n;
const B = 0x5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604bn;
const G = [0x6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296n,
  0x4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5n];
const mod = (a, m) => ((a % m) + m) % m;
function inv(a, m) {
  let [x0, x1, r0, r1] = [0n, 1n, m, mod(a, m)];
  while (r1) { const q = r0 / r1; [x0, x1] = [x1, x0 - q * x1]; [r0, r1] = [r1, r0 - q * r1]; }
  return mod(x0, m);
}
function pow(b, e, m) { let r = 1n; for (b = mod(b, m); e > 0n; e >>= 1n, b = b * b % m) if (e & 1n) r = r * b % m; return r; }
function add(p1, p2) {
  if (!p1) return p2;
  if (!p2) return p1;
  const [x1, y1] = p1, [x2, y2] = p2;
  let l;
  if (x1 === x2) {
    if (mod(y1 + y2, P) === 0n) return null;
    l = mod((3n * x1 * x1 + A) * inv(2n * y1, P), P);
  } else {
    l = mod((y2 - y1) * inv(x2 - x1, P), P);
  }
  const x3 = mod(l * l - x1 - x2, P);
  return [x3, mod(l * (x1 - x3) - y1, P)];
}
function mul(k, pt) { let r = null; for (let q = pt; k > 0n; k >>= 1n, q = add(q, q)) if (k & 1n) r = add(r, q); return r; }

/** The public keys (x, y as bytes) that one assertion's signature fits: usually two. */
export async function candidates({ authenticatorData, clientDataJSON, signature }) {
  const z = big(await sha256(concat(authenticatorData, await sha256(clientDataJSON))));
  const { r, s } = parseDer(signature);
  const y2 = mod(r * r * r + A * r + B, P);
  const y = pow(y2, (P + 1n) / 4n, P); // P = 3 mod 4
  if (y * y % P !== y2) return [];
  const ri = inv(r, N), out = [];
  for (const R of [[r, y], [r, P - y]]) {
    const Q = mul(ri, add(mul(s, R), mul(mod(-z, N), G)));
    if (Q) out.push({ x: word(Q[0]), y: word(Q[1]) });
  }
  return out;
}

/** The one public key that fits two assertions from the same passkey, checked with WebCrypto. */
export async function keyFrom(a1, a2) {
  const c2 = (await candidates(a2)).map((k) => hex(k.x) + hex(k.y));
  const both = (await candidates(a1)).filter((k) => c2.includes(hex(k.x) + hex(k.y)));
  if (both.length !== 1) throw new Error("these signatures don't come from one key");
  const k = both[0], key = await importKey(k.x, k.y);
  for (const a of [a1, a2]) {
    const { r, s } = parseDer(a.signature);
    const ok = await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, key, concat(word(r), word(s)),
      concat(a.authenticatorData, await sha256(a.clientDataJSON)));
    if (!ok) throw new Error("the recovered key doesn't verify");
  }
  return k;
}

// ------------------------------------------------------------------ the browser's side

const STORE = "diykey.passkey";

/** The passkey this browser made, if it remembers one: { id, x, y } as hex. */
export function saved() {
  try { return JSON.parse(localStorage.getItem(STORE) || "null"); } catch { return null; }
}
function remember(k) {
  try { localStorage.setItem(STORE, JSON.stringify(k)); } catch { /* private window: it works until reload */ }
  return k;
}
export function forget() {
  try { localStorage.removeItem(STORE); } catch { /* nothing kept */ }
}

/**
 * Makes a passkey for this site and remembers its credential id and public key in this browser,
 * to check its signatures. Forget them and the passkey stays in the keychain, but this page can't
 * check its signatures until it makes a new one.
 */
export async function create() {
  const rpId = location.hostname;
  const cred = await navigator.credentials.create({ publicKey: {
    rp: { name: "diykey", id: rpId },
    user: { id: crypto.getRandomValues(new Uint8Array(16)), name: "diykey virtual chip", displayName: "diykey virtual chip" },
    challenge: crypto.getRandomValues(new Uint8Array(32)),
    pubKeyCredParams: [{ type: "public-key", alg: -7 }],
    authenticatorSelection: { residentKey: "preferred", userVerification: "required" },
    attestation: "none",
  } });
  const { id, x, y } = parseAttestation(new Uint8Array(cred.response.attestationObject));
  return remember({ id: hex(id), x: hex(x), y: hex(y) });
}

/**
 * Asks the passkey to sign `hash` (the emulator's, 0x…). Call it straight from a key press or a
 * tap: Safari asks for one. Returns the assertion's parts as bytes.
 */
export function sign(hash, key) {
  return navigator.credentials.get({ publicKey: {
    challenge: fromHex(hash),
    rpId: location.hostname,
    allowCredentials: [{ type: "public-key", id: fromHex(key.id) }],
    userVerification: "required",
  } }).then((a) => ({
    authenticatorData: new Uint8Array(a.response.authenticatorData),
    clientDataJSON: new Uint8Array(a.response.clientDataJSON),
    signature: new Uint8Array(a.response.signature),
  }));
}

const parts = (a) => ({
  authenticatorData: new Uint8Array(a.response.authenticatorData),
  clientDataJSON: new Uint8Array(a.response.clientDataJSON),
  signature: new Uint8Array(a.response.signature),
});

/** Using a passkey made before, step 1: the browser lists this site's passkeys and the person picks
 * one, which signs a random challenge. Call it from a tap. */
export async function find() {
  const a = await navigator.credentials.get({ publicKey: {
    challenge: crypto.getRandomValues(new Uint8Array(32)), rpId: location.hostname, userVerification: "required",
  } });
  return { id: new Uint8Array(a.rawId), ...parts(a) };
}

/** Step 2, from a second tap (Safari asks for one per prompt): the same passkey signs once more,
 * and the two signatures give its public key. */
export async function confirm(found) {
  const a = await navigator.credentials.get({ publicKey: {
    challenge: crypto.getRandomValues(new Uint8Array(32)), rpId: location.hostname, userVerification: "required",
    allowCredentials: [{ type: "public-key", id: found.id }],
  } });
  const k = await keyFrom(found, parts(a));
  return remember({ id: hex(found.id), x: hex(k.x), y: hex(k.y) });
}

export { hex as toHex, fromHex };
