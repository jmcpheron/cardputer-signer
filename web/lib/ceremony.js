// The key ceremony as the page checks it (docs/CEREMONY.md). The ceremony image prints what it did,
// a line a step; the page works out the same things on its own from what it knows and compares:
// the config CRC from the factory config P1 read and the reviewed table, the data CRC from the
// record, each test transaction's Safe hash, each test signature (WebCrypto, P-256), and the
// key's Safe signer address. web/test.mjs checks this against a rehearsal on a virtual chip
// (web/screens/ceremony-rehearsal.txt).
import { fromHex, hex, safeTxHash } from "./safe.js";
import { b64url, importKey } from "./passkey.js";
import { signerAddress } from "./signer.js";

/** The rules, diykey v1, the same 128 bytes as core/src/ceremony.rs. 0-15 and 84-87 are the
 * chip's own. */
export const TABLE = fromHex(
  "012300000000600000000000ee000000c0000000a12f9f8f9f8f9f8f9f8f9f8f" +
  "9f8f9f8f1f8f9f8f9f8f9f8f9f8f9f8f9f8f9f8fffffffff00000000ffffffff" +
  "000000000000000000000000000000000000000000005555ffff000000000000" +
  "33001c001c001c001c001c001c001c001c001c001c001c001c001c001c001c00");
/** What the device says it is, in the message it signs (core/src/webauthn.rs). */
export const RP_ID = "diykey.app";
export const ORIGIN = "https://diykey.app";
const ZERO = "0x0000000000000000000000000000000000000000";
const TEST_DATA = "diykey key ceremony: a test signature";
const ORDER = ["P1", "P2", "P3", "P4", "P5", "P6", "K1", "K2", "K3", "K4"];

/** The chip's CRC-16 (poly 0x8005, bits taken low first), as the 16-bit number Lock takes. */
export function crc16(bytes, crc = 0) {
  for (const b of bytes) {
    for (let i = 0; i < 8; i++) {
      const bit = (b >> i) & 1, top = (crc >> 15) & 1;
      crc = (crc << 1) & 0xffff;
      if (bit !== top) crc ^= 0x8005;
    }
  }
  return crc;
}
const h4 = (n) => n.toString(16).padStart(4, "0");

/** The config CRC P3 locks with: the factory config's first 16 bytes, then the table. */
export function configCrc(factory) {
  const image = TABLE.slice();
  image.set(factory.slice(0, 16));
  return h4(crc16(image));
}

/** The data CRC P5 locks with: slots 1-15, all zeros (7 × 36, 416, 7 × 72 bytes), then the record. */
export function dataCrc(record) {
  return h4(crc16(record, crc16(new Uint8Array(7 * 36 + 416 + 7 * 72))));
}

/** Test transaction n's Safe hash: nothing to address zero on Base Sepolia, for a Safe at address
 * zero, which no Safe can be. */
export function testHash(n) {
  return safeTxHash({
    chain: "84532", safe: ZERO, to: ZERO, value: "0", data: "0x" + hex(new TextEncoder().encode(TEST_DATA)), op: "0",
    nonce: String(n), safetxgas: "0", basegas: "0", gasprice: "0", gastoken: ZERO, refundreceiver: ZERO,
  }).hash;
}

const sha256 = async (b) => new Uint8Array(await crypto.subtle.digest("SHA-256", b));
const cat = (...p) => { const o = new Uint8Array(p.reduce((n, x) => n + x.length, 0)); let i = 0; for (const x of p) { o.set(x, i); i += x.length; } return o; };

/** The message the chip signs for `hash`: authenticatorData ‖ sha256(clientDataJSON). */
export async function message(hash) {
  const auth = cat(await sha256(new TextEncoder().encode(RP_ID)), Uint8Array.of(0x05, 0, 0, 0, 0));
  const client = `{"type":"webauthn.get","challenge":"${b64url(fromHex(hash))}","origin":"${ORIGIN}"}`;
  return cat(auth, await sha256(new TextEncoder().encode(client)));
}

/** Whether (r, s) is the key's P-256 signature over test transaction n's message. */
export async function checkSig(pubkey, n, r, s) {
  try {
    const key = await importKey(fromHex(pubkey.slice(0, 64)), fromHex(pubkey.slice(64)));
    return await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, key, fromHex(r + s), await message(testHash(n)));
  } catch {
    return false; // not a point on the curve
  }
}

/**
 * Reads the ceremony's lines as they come and keeps what it has checked. `feed` takes one line and
 * returns a promise that settles once its checks are done; lines are read in the order fed.
 */
export function reader() {
  const st = { checks: [], lines: [] };
  const check = (step, what, ok, detail = "") => {
    st.checks = st.checks.filter((c) => !(c.step === step && c.what === what));
    st.checks.push({ step, what, ok, detail });
    st.checks.sort((a, b) => ORDER.indexOf(a.step) - ORDER.indexOf(b.step));
  };
  let queue = Promise.resolve();
  st.feed = (line) => (queue = queue.then(() => read(line)));
  const read = async (line) => {
    let m;
    if ((m = line.match(/^diykey-ceremony: commit ([0-9a-f]{40})$/))) {
      st.commit = m[1];
    } else if ((m = line.match(/^chip: serial \[([0-9a-f, ]+)\]/))) {
      st.serial = m[1].replace(/[ ,]/g, "");
    } else if (!line.match(/^(p\d|k\d|stop): /)) {
      return;
    }
    st.lines.push(line);
    if ((m = line.match(/^p1: factory config 0x([0-9a-f]{256})$/))) {
      st.factory = fromHex(m[1]);
      st.pageConfigCrc = configCrc(st.factory);
    } else if ((m = line.match(/^p2: .*config crc ([0-9a-f]{4})$/))) {
      st.configCrc = m[1];
      check("P2", "Config CRC", st.pageConfigCrc ? m[1] === st.pageConfigCrc : null,
        st.pageConfigCrc ? `the Cardputer's ${m[1]}, this page's ${st.pageConfigCrc}` : "the factory config didn't reach this page");
    } else if ((m = line.match(/^p4: .*record 0x([0-9a-f]{128}); data crc ([0-9a-f]{4})$/))) {
      const record = fromHex(m[1]);
      st.record = m[1];
      st.dataCrc = m[2];
      const want = dataCrc(record);
      check("P4", "Data CRC", m[2] === want, `the Cardputer's ${m[2]}, this page's ${want}`);
      const serialOk = st.serial ? hex(record.slice(4, 13)) === st.serial : null;
      check("P4", "The record names this chip", serialOk, st.serial ? `serial ${hex(record.slice(4, 13))}` : "");
    } else if ((m = line.match(/^(k1|k4): pubkey 0x([0-9a-f]{128})$/))) {
      st.pubkey = m[2];
      st.sigs = {};
      try {
        await importKey(fromHex(m[2].slice(0, 64)), fromHex(m[2].slice(64)));
        check("K1", "The public key is a point on P-256", true);
      } catch {
        check("K1", "The public key is a point on P-256", false);
      }
    } else if ((m = line.match(/^(k2|k3): sig (\d) hash 0x([0-9a-f]{64}) r 0x([0-9a-f]{64}) s 0x([0-9a-f]{64})$/))) {
      const n = Number(m[2]);
      const hashOk = m[3] === testHash(n).slice(2);
      const sigOk = !!st.pubkey && hashOk && await checkSig(st.pubkey, n, m[4], m[5]);
      check(m[1].toUpperCase(), `Test signature ${n}`, sigOk,
        !hashOk ? "not test transaction " + n + "'s hash" : sigOk ? "valid, checked with WebCrypto" : "doesn't verify");
    } else if ((m = line.match(/^k2: .*counter0 (\d+) -> (\d+)$/))) {
      check("K2", "Counter 0 went up by the two signatures", Number(m[2]) - Number(m[1]) === 2, `${m[1]} to ${m[2]}`);
    } else if ((m = line.match(/^k4: address (0x[0-9a-fA-F]{40})$/))) {
      st.address = m[1];
      const mine = st.pubkey ? signerAddress(st.pubkey.slice(0, 64), st.pubkey.slice(64)) : null;
      st.pageAddress = mine;
      check("K4", "The address", mine ? mine === m[1] : null, mine ? `this page's ${mine}` : "no public key reached this page");
    } else if ((m = line.match(/^stop: (.*)$/))) {
      st.stop = m[1];
    } else if (line.startsWith("chip: ")) {
      st.stop = null;
    }
  };
  return st;
}
