// The emulator: signer.wasm (built from wasm/, the signer's core) as a Cardputer that answers in
// the same lines as the real one. The page writes a line or a key in; lines come back.

const KEYS = { up: 1, down: 2, left: 3, right: 4, enter: 5, del: 6 };

export async function load(bytes) {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const x = instance.exports;
  const mem = () => new Uint8Array(x.memory.buffer);
  // Ask for a buffer's address before taking the view: the first call into the wasm sets up its
  // buffers and grows its memory, which leaves an older view of it empty.
  const lines = (n) => { const at = x.out_buf(); return new TextDecoder().decode(mem().slice(at, at + n)).split("\n").filter(Boolean); };
  const w = x.width(), h = x.height();
  return {
    w, h,
    boot: () => lines(x.boot()),
    send(line) {
      const b = new TextEncoder().encode(line);
      if (b.length <= x.line_cap()) { const at = x.line_buf(); mem().set(b, at); }
      return lines(x.submit(b.length));
    },
    key: (name) => lines(x.key(KEYS[name])),
    /** The passkey's credential id (hex), or null for none. */
    setPasskey(idHex) {
      const id = idHex ? Uint8Array.from(idHex.slice(0, 16).match(/../g), (v) => parseInt(v, 16)) : new Uint8Array();
      const at = x.line_buf();
      mem().set(id, at);
      return lines(x.set_passkey(id.length));
    },
    signResult: (ok) => lines(x.sign_result(ok ? 1 : 0)),
    /** The screen, 240 × 135 RGBA. */
    pixels() {
      x.draw();
      return new Uint8ClampedArray(x.memory.buffer.slice(x.rgba_buf(), x.rgba_buf() + w * h * 4));
    },
  };
}
