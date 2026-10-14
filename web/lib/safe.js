// The Safe transaction hash, worked out in the browser: the hash the computer claims (`expect`).
// The Cardputer works out its own and only compares; it never signs this one. Keccak-256 in plain
// JavaScript (BigInt lanes), and the EIP-712 steps of core/src/safe_tx.rs. web/test.mjs checks
// both against vectors/safe_tx.json.

const MASK = (1n << 64n) - 1n;
// Rotation offsets, lane x + 5y.
const ROT = [0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14]
  .map(BigInt);
// Round constants, from the spec's LFSR.
const RC = (() => {
  const out = [];
  let r = 1;
  for (let i = 0; i < 24; i++) {
    let rc = 0n;
    for (let j = 0; j < 7; j++) {
      r = ((r << 1) ^ ((r >> 7) * 0x71)) % 256;
      if (r & 2) rc ^= 1n << ((1n << BigInt(j)) - 1n);
    }
    out.push(rc);
  }
  return out;
})();

const rotl = (v, n) => (n === 0n ? v : ((v << n) | (v >> (64n - n))) & MASK);

function permute(s) {
  const c = new Array(5), b = new Array(25);
  for (let round = 0; round < 24; round++) {
    for (let x = 0; x < 5; x++) c[x] = s[x] ^ s[x + 5] ^ s[x + 10] ^ s[x + 15] ^ s[x + 20];
    for (let x = 0; x < 5; x++) {
      const d = c[(x + 4) % 5] ^ rotl(c[(x + 1) % 5], 1n);
      for (let y = 0; y < 25; y += 5) s[x + y] ^= d;
    }
    for (let x = 0; x < 5; x++) {
      for (let y = 0; y < 5; y++) b[y + 5 * ((2 * x + 3 * y) % 5)] = rotl(s[x + 5 * y], ROT[x + 5 * y]);
    }
    for (let y = 0; y < 25; y += 5) {
      for (let x = 0; x < 5; x++) s[x + y] = b[x + y] ^ (~b[(x + 1) % 5 + y] & MASK & b[(x + 2) % 5 + y]);
    }
    s[0] ^= RC[round];
  }
}

/** Keccak-256 (Ethereum's, not SHA3-256) of a Uint8Array. */
export function keccak256(bytes) {
  const rate = 136;
  const padded = new Uint8Array(Math.floor(bytes.length / rate) * rate + rate);
  padded.set(bytes);
  padded[bytes.length] ^= 0x01;
  padded[padded.length - 1] ^= 0x80;
  const s = new Array(25).fill(0n);
  for (let off = 0; off < padded.length; off += rate) {
    for (let i = 0; i < rate / 8; i++) {
      let lane = 0n;
      for (let k = 7; k >= 0; k--) lane = (lane << 8n) | BigInt(padded[off + i * 8 + k]);
      s[i] ^= lane;
    }
    permute(s);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 4; i++) {
    for (let k = 0; k < 8; k++) out[i * 8 + k] = Number((s[i] >> BigInt(8 * k)) & 0xffn);
  }
  return out;
}

export const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function fromHex(h) {
  h = h.startsWith("0x") ? h.slice(2) : h;
  if (h.length % 2 || /[^0-9a-fA-F]/.test(h)) throw new Error("not hex: " + h.slice(0, 20));
  return Uint8Array.from(h.match(/../g) || [], (x) => parseInt(x, 16));
}

const concat = (...parts) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
};
const ascii = (s) => new TextEncoder().encode(s);
function uint(v) {
  let n = BigInt(v);
  if (n < 0n || n >= 1n << 256n) throw new Error("out of range: " + v);
  const out = new Uint8Array(32);
  for (let i = 31; i >= 0; i--) { out[i] = Number(n & 0xffn); n >>= 8n; }
  return out;
}
function addr(a) {
  const b = fromHex(a);
  if (b.length !== 20) throw new Error("not an address: " + a);
  return concat(new Uint8Array(12), b);
}

export const DOMAIN_TYPE = "EIP712Domain(uint256 chainId,address verifyingContract)";
export const SAFE_TX_TYPE = "SafeTx(address to,uint256 value,bytes data,uint8 operation,uint256 safeTxGas," +
  "uint256 baseGas,uint256 gasPrice,address gasToken,address refundReceiver,uint256 nonce)";
const ZERO = "0x0000000000000000000000000000000000000000";

/** A vector's tx with every optional field filled in, as send.py fills them. */
export function fields(v) {
  const t = v.tx;
  return {
    chain: String(v.chainId), safe: v.safe, to: t.to, value: String(t.value ?? "0"), data: t.data || "0x",
    op: String(t.operation ?? 0), nonce: String(t.nonce), safetxgas: String(t.safeTxGas ?? "0"),
    basegas: String(t.baseGas ?? "0"), gasprice: String(t.gasPrice ?? "0"), gastoken: t.gasToken || ZERO,
    refundreceiver: t.refundReceiver || ZERO,
  };
}

/** The three EIP-712 steps for Safe >= 1.3.0: domain separator, struct hash, safeTxHash. */
export function safeTxHash(f) {
  if (f.op !== "0" && f.op !== "1") throw new Error("operation must be 0 or 1");
  const domain = keccak256(concat(keccak256(ascii(DOMAIN_TYPE)), uint(f.chain), addr(f.safe)));
  const struct = keccak256(concat(
    keccak256(ascii(SAFE_TX_TYPE)), addr(f.to), uint(f.value), keccak256(fromHex(f.data)), uint(f.op),
    uint(f.safetxgas), uint(f.basegas), uint(f.gasprice), addr(f.gastoken), addr(f.refundreceiver), uint(f.nonce)));
  const hash = keccak256(concat(Uint8Array.of(0x19, 0x01), domain, struct));
  return { domain: "0x" + hex(domain), struct: "0x" + hex(struct), hash: "0x" + hex(hash) };
}

/** The line the device reads (core/src/proto.rs), as host/send.py builds it. */
export function txLine(f, expect) {
  const parts = ["chain", "safe", "to", "value", "data", "op", "nonce", "safetxgas", "basegas", "gasprice",
    "gastoken", "refundreceiver"].map((k) => `${k}=${f[k]}`);
  if (expect) parts.push(`expect=${expect}`);
  return "tx " + parts.join(" ");
}

/** A hash one hex digit off, for the tamper demo: the last digit changed, as send.py --tamper. */
export const tamper = (h) => h.slice(0, -1) + (h.endsWith("0") ? "1" : "0");
