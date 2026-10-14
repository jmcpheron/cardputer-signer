// A P-256 key's address on chain: the Safe passkey signer for it. A P-256 key can't be an Ethereum
// account by itself, so Safe puts each one behind its own small contract, a SafeWebAuthnSignerProxy
// (safe-modules passkey v0.2.1), deployed by Safe's factory at an address fixed by the key. That
// address is what a Safe lists as its owner, and it is the same on every chain the factory is on.
// It only checks signatures for the Safe: it isn't a wallet and can't use ETH or tokens sent to it.
// web/test.mjs checks this against vectors/webauthn.json, whose addresses came from the factory.
import { fromHex, hex, keccak256 } from "./safe.js";

export const SIGNER = {
  factory: "0x1d31f259ee307358a26dfb23eb365939e8641195",
  singleton: "0x4e27b51350e6c2083ee19011120f50dafec5ca50",
  // the P-256 precompile at 0x100 first, then the Daimo verifier: part of the address
  verifiers: "0x0100c2b78104907f722dabac4c69f826a522b2754de4",
  // SafeWebAuthnSignerProxy's creation code, as the factory deploys it
  proxyCode: "0x610100346100ad57601f6101b538819003918201601f19168301916001600160401b038311848410176100b2578084926080946040528339810103126100ad578051906001600160a01b03821682036100ad5760208101516040820151606090920151926001600160b01b03841684036100ad5760805260a05260c05260e05260405160ec90816100c98239608051816082015260a05181604d015260c051816027015260e0518160010152f35b600080fd5b634e487b7160e01b600052604160045260246000fdfe7f000000000000000000000000000000000000000000000000000000000000000060b63601527f000000000000000000000000000000000000000000000000000000000000000060a03601527f000000000000000000000000000000000000000000000000000000000000000036608001523660006080376000806056360160807f00000000000000000000000000000000000000000000000000000000000000005af43d600060803e60b1573d6080fd5b3d6080f3fea26469706673582212201660515548d15702d720bbc046b457ca85e941a4559ab9f9518488e4c82e5ee964736f6c634300081a0033",
};

const word = (b) => { const w = new Uint8Array(32); w.set(b, 32 - b.length); return w; };
const cat = (...p) => { const o = new Uint8Array(p.reduce((n, x) => n + x.length, 0)); let i = 0; for (const x of p) { o.set(x, i); i += x.length; } return o; };

/** The signer's address for a P-256 key (x, y as bytes or hex): CREATE2 by the factory, salt 0,
 * over the proxy's creation code and its arguments (singleton, x, y, verifiers). EIP-55 cased. */
export function signerAddress(x, y) {
  const b = (v) => (typeof v === "string" ? fromHex(v) : v);
  const init = cat(fromHex(SIGNER.proxyCode), word(fromHex(SIGNER.singleton)), word(b(x)), word(b(y)), word(fromHex(SIGNER.verifiers)));
  const h = keccak256(cat(Uint8Array.of(0xff), fromHex(SIGNER.factory), new Uint8Array(32), keccak256(init)));
  return checksum(hex(h.slice(12)));
}

/** EIP-55: the address with its letters cased by its own keccak. */
export function checksum(addr) {
  const a = addr.replace(/^0x/, "").toLowerCase();
  const h = hex(keccak256(new TextEncoder().encode(a)));
  return "0x" + [...a].map((c, i) => (parseInt(h[i], 16) >= 8 ? c.toUpperCase() : c)).join("");
}

/** A short fingerprint of the public key: the first 8 bytes of sha256(x ‖ y), in four groups. */
export async function fingerprint(x, y) {
  const b = (v) => (typeof v === "string" ? fromHex(v) : v);
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", cat(b(x), b(y))));
  return hex(d.slice(0, 8)).match(/.{4}/g).join(" ");
}
