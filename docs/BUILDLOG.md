# Build log

How mine came together, with what ran and what didn't. Newest last.

## 2026-10-11: the Cardputer's screen and keyboard

A Cardputer v1.1 on the bench, under MicroPython 1.26.1, to find out how its screen and keyboard
are wired. Checked by eye and with a scan of the keyboard matrix:

- The screen is an ST7789V2. Its 240 × 135 of glass sits at x = 40, y = 52 in the controller's RAM,
  landscape with MADCTL 0x60. SPI: MOSI 35, SCK 36, DC 34, RST 33, CS 37, backlight 38.
- The keyboard is a 74HC138 on G8, G9 and G11 that picks one of 8 lines. G13, G15 and G3–G7 read
  that line's keys, low while a key is down.
- `;` and `.` are up and down, `,` and `/` left and right, then Enter and Del. Not G0: held at reset,
  it starts the ROM bootloader.

## 2026-10-12: the key on the Grove port

The ATECC608 went on the Grove port through the AMS1117-3.3 module, wired as in
[hardware/](../hardware/README.md): G2 to SDA, G1 to SCL, the chip on 3.3 V. It answered at 0x60.

Before this Rust firmware, I ran an earlier MicroPython version on the Cardputer. It needed a custom
MicroPython build to fit in the board's RAM: modules run from flash (ROMFS) and a heap that never
grows. It reached two Safes on Base Sepolia over Wi-Fi. Its code isn't in this repository.

## 2026-10-12: milestone 1 in Rust

Bare-metal Rust (`esp-hal` 1.2.2, no OS). `core/` holds everything that isn't hardware and runs on
the laptop. `firmware/` is the Cardputer.

Flashed with `cargo run --release`. The serial log showed the chip read the same way the MicroPython
driver reads it:
- serial `0123f3acfd2a826bee`, revision `00006002` (a 608A)
- config zone locked, data zone open, counter 0 at 23

The screen and keys looked right by eye.

## 2026-10-12: milestone 2, a transaction over USB

A transaction arrives over USB as one line of text. The device works out its Safe hash itself and
shows what the transaction does, a page at a time. Del rejects. Nothing signs yet.

What ran on the device:
- `host/send.py eth-send-base-sepolia` got back the vector's own safeTxHash, worked out on the device.
- A second transaction while one was on screen got `= busy`.
- `sign …` got `= error unknown command`.
- A short address and `op=7` were refused.

One fix on the way: esp-println gave up on a write so soon that macOS lost bytes of a hash. Replies
now go through the firmware's own writer, which waits up to 200 ms.

Not yet checked by hand: paging, Reject, and the red hash page for `--tamper`.

## 2026-10-13: this repository

The firmware, the wiring and these notes moved into their own repository. The firmware is the same
code as on the bench; only comments and the vectors' descriptions changed. On the laptop, `cargo
test` passed (15 tests) and the firmware built at 160 KB. It wasn't flashed again.

## 2026-10-13: a web page that talks to the Cardputer

`web/` sends the demo transactions to the Cardputer over USB (Web Serial) and can flash it
(esptool-js). On the board: `ping` came back `= pong cardputer-signer m2`, and
`eth-send-base-sepolia` came back with the vector's own safeTxHash, `…085c2b6d`, the same one the
page works out. Opening the port didn't reset the board. Then the page itself, connected and sending,
checked by hand. Flashing from the page wasn't tried.

## 2026-10-13: diykey, an emulator and passkeys

The repository is now diykey, for diykey.app. `core` compiles to WebAssembly (`wasm/`, 125 KB) and
runs as an emulator in the page, with the firmware's states and the same reply lines. Its key is a
passkey: Enter on the hash page asks it to sign the hash the emulator worked out, in the WebAuthn
format Safe's passkey signer checks ([PASSKEY.md](PASSKEY.md)), and the page verifies the signature.

Checked: `node web/test.mjs` (the emulator's hash for all 17 vectors, its signing rules, the passkey
parsing and verification with Node as the authenticator), and `node web/e2e.mjs` in Chrome with a
virtual authenticator: a passkey made, a transaction signed and verified, a wrong claim not signed.
Not checked yet: a real phone's passkey, and the Safe encoding against the contract onchain.

core gained a passkey key state, a Sign footer and a Signed screen, which the firmware doesn't use.
All 27 of the firmware's screens came out the same, byte for byte. The firmware image's hash
changed (`06ec4ab9…`), from code layout; the board didn't need flashing.

## 2026-10-14: the key's address, and the bench

The firmware now reads slot 0's public key when it reads the chip (GenKey mode 0, which only works
out the public key and changes nothing) and reports it as `chip: pubkey0 0x…`. A test pins the
packet to `0307400000000005`, the bytes the MicroPython driver sends, and pins the mode at 0 for
every slot: mode 0x04 would make a new key. Flashed from the page and run with a second chip on the
Grove port (serial starting `01233859`): config and data zones open, and no key in slot 0, so the
page shows no address. The first chip (`0123f3acfd2a826bee`) has a key in slot 0 from the
MicroPython version, whose Safe signer address was `0x2843705391b7E2cf06b0b964b532Ad9891535E53`;
not yet read with this firmware.

The page shows the key that's attached and the address it signs as on chain (Safe's passkey signer
for that key, [PASSKEY.md](PASSKEY.md)). `web/test.mjs` checks the address and the signature
encoding against `vectors/webauthn.json`. The top of the page is now the bench, laid out as on the
desk: the key, the Grove wires, the Cardputer with the live screen and a keyboard that works, and the
USB cable into "this computer", where the transactions are picked and sent.

The key is now the panel at the left, with its wires (or the browser's WebAuthn) drawn into the
Grove port, and the Cardputer takes the space the drawing had: its screen is drawn at about 1:1 on a
wide window. A bug found on the bench: with a passkey saved in the browser, the page stopped at
startup and the Connect button did nothing. The emulator's first call sets up its buffers and grows
its memory, and `web/lib/emu.js` had taken its view of memory before that call. Fixed, with a test
in `web/test.mjs` and a reload step in `web/e2e.mjs`.

Every line now runs along the cable as it goes: down the USB cable in blue, back up it in green,
and over the key's wires when the signer reads its key. "This computer" shows what went down and
what came back, with the cable plugged in between, and the two hashes and the signature sit in a
band under the bench.

## Next

- Milestone 3: hold to sign with the chip, over the hash the device worked out.
- The key ceremony ([CEREMONY.md](CEREMONY.md)), on a spare chip first.
- A case for the regulator and the chip.
