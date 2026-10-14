#!/bin/sh
# Builds what web/ serves but git doesn't keep:
#   web/screens/   every page of every vector, drawn by core exactly as the Cardputer draws them,
#                  and reviews.json: what the device says about each (summary, items, level, pages)
#   web/firmware/  the firmware as one image for address 0 (bootloader, partition table, app),
#                  and firmware.json with its size, SHA-256 and the commit it was built from
# Needs Rust, espup's toolchain (~/export-esp.sh) and espflash, as in the README.
set -eu
cd "$(dirname "$0")/.."

rm -rf web/screens && mkdir -p web/screens web/firmware
(cd core && cargo run --locked --quiet --example screens -- ../web/screens all) >/dev/null

. "$HOME/export-esp.sh"
(cd firmware && cargo build --release --locked --quiet)
espflash save-image --chip esp32s3 --flash-size 8mb --merge --skip-padding \
  firmware/target/xtensa-esp32s3-none-elf/release/firmware web/firmware/cardputer-signer.bin >/dev/null

COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
DIRTY=$(git status --porcelain -- core/src core/Cargo.toml core/Cargo.lock firmware fonts 2>/dev/null | grep -q . && echo true || echo false)
python3 - "$COMMIT" "$DIRTY" <<'EOF'
import glob, hashlib, json, struct, sys, time, zlib

# The example writes PNGs with stored (uncompressed) deflate blocks: recompress them, same pixels.
for f in sorted(glob.glob("web/screens/*.png")):
    b = open(f, "rb").read()
    i, chunks, idat = 8, [], b""
    while i < len(b):
        n, t = struct.unpack(">I4s", b[i:i + 8]); d = b[i + 8:i + 8 + n]; i += 12 + n
        if t == b"IDAT": idat += d
        else: chunks.append((t, d))
    raw = zlib.decompress(idat)
    out = b"\x89PNG\r\n\x1a\n"
    for t, d in chunks:
        if t == b"IEND":
            z = zlib.compress(raw, 9)
            out += struct.pack(">I", len(z)) + b"IDAT" + z + struct.pack(">I", zlib.crc32(b"IDAT" + z))
        out += struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    open(f, "wb").write(out)

img = open("web/firmware/cardputer-signer.bin", "rb").read()
json.dump({
    "file": "cardputer-signer.bin", "address": 0, "size": len(img),
    "sha256": hashlib.sha256(img).hexdigest(), "commit": sys.argv[1], "dirty": sys.argv[2] == "true",
    "built": time.strftime("%Y-%m-%d %H:%M", time.localtime()), "chip": "esp32s3", "flashSize": "8MB",
}, open("web/firmware/firmware.json", "w"), indent=1)
print(f"web/screens: {len(glob.glob('web/screens/*.png'))} pages for {len(json.load(open('web/screens/reviews.json')))} transactions")
print(f"web/firmware: {len(img):,} bytes, sha256 {hashlib.sha256(img).hexdigest()}")
EOF
