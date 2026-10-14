// Blockies identicons (github.com/ethereum/blockies), as core/src/blockies.rs draws them, so the
// page shows the same picture as the Cardputer for the same seed. web/test.mjs checks this
// against vectors/blockies.json.

function rgb(h, s, l) {
  // CSS hsl(): the hue wraps, saturation and lightness clamp; channels round half to even, as
  // the device does.
  h = h % 360;
  s = Math.min(Math.max(s, 0), 100) / 100;
  l = Math.min(Math.max(l, 0), 100) / 100;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const hp = h / 60;
  const x = c * (1 - Math.abs(hp - 2 * Math.floor(hp / 2) - 1));
  const [r, g, b] = hp < 1 ? [c, x, 0] : hp < 2 ? [x, c, 0] : hp < 3 ? [0, c, x]
    : hp < 4 ? [0, x, c] : hp < 5 ? [x, 0, c] : [c, 0, x];
  const m = l - c / 2;
  const ch = (v) => {
    const t = (v + m) * 255, f = Math.floor(t), d = t - f;
    const r = d > 0.5 ? f + 1 : d < 0.5 ? f : f % 2 === 0 ? f : f + 1;
    return Math.min(Math.max(r, 0), 255);
  };
  return [ch(r), ch(g), ch(b)];
}

export function build(seed) {
  const rs = [0, 0, 0, 0];
  for (let i = 0; i < seed.length; i++) {
    rs[i % 4] = ((rs[i % 4] << 5) - rs[i % 4]) + seed.charCodeAt(i);
  }
  // The JS rand()'s numerator: rand() is this / 2**31.
  const u = () => {
    const t = rs[0] ^ (rs[0] << 11);
    rs[0] = rs[1]; rs[1] = rs[2]; rs[2] = rs[3];
    rs[3] = rs[3] ^ (rs[3] >> 19) ^ t ^ (t >> 8);
    return rs[3] >>> 0;
  };
  const hsl = () => {
    const h = Math.floor(u() * 360 / 2 ** 31);
    const s = u() * 60 / 2 ** 31 + 40;
    const l = (u() + u() + u() + u()) * 25 / 2 ** 31;
    return rgb(h, s, l);
  };
  const color = hsl(), bg = hsl(), spot = hsl();
  const cells = [];
  for (let row = 0; row < 8; row++) {
    const half = [0, 1, 2, 3].map(() => Math.floor(u() * 23 / (10 * 2 ** 31)));
    cells.push(...half, ...half.slice().reverse());
  }
  return { cells, color, bg, spot };
}

/** A blockie on a canvas, `size` CSS pixels square, crisp at any screen density. */
export function canvas(seed, size) {
  const b = build(seed);
  const c = document.createElement("canvas");
  c.width = c.height = 8;
  c.className = "blockie";
  c.style.width = c.style.height = size + "px";
  const g = c.getContext("2d");
  const img = g.createImageData(8, 8);
  b.cells.forEach((v, i) => {
    const [r, gg, bb] = v === 0 ? b.bg : v === 1 ? b.color : b.spot;
    img.data.set([r, gg, bb, 255], i * 4);
  });
  g.putImageData(img, 0, 0);
  return c;
}
