// The page's own hash and blockies against the vectors: node web/test.mjs
import { readFileSync } from "node:fs";
import { build } from "./lib/blockies.js";
import { fields, hex, keccak256, safeTxHash, tamper, txLine } from "./lib/safe.js";

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

console.log(failed ? `${failed} failed` : `ok: keccak, ${vectors.length} Safe hashes, ${blockies.length} blockies`);
process.exit(failed ? 1 : 0);
