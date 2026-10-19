# The hardware

Three parts and some wires: a Cardputer, a 3.3 V regulator and an ATECC608 breakout on the Grove
port. No soldering if your regulator module and breakout come with header pins fitted.

![Wiring: Grove 5 V through an AMS1117-3.3 to the ATECC608's VIN; G2 to SDA, G1 to SCL](wiring.svg)

## Parts

| part | what I used | notes |
|---|---|---|
| M5Stack Cardputer v1.1 | the v1.1 (Stamp-S3A: ESP32-S3, 8 MB flash, no PSRAM) | The pins in the firmware were checked on a v1.1 only. Not the Cardputer-Adv ([below](#shopping-list)). |
| ATECC608 breakout | an ATECC608A on a breakout board, such as Adafruit's STEMMA QT board (4314) | Any blank ATECC608A or 608B at I2C address 0x60. The board should have its own SDA and SCL pull-ups. Adafruit's has them. |
| 3.3 V regulator | HiLetgo AMS1117-3.3 module (a pack of 10) | Sold as a "DC-DC step-down buck module"; it's really an AMS1117 linear regulator, up to 800 mA. Grove's 5 V in, 3.3 V out to the chip. |
| Grove cable | Grove (HY2.0-4P) to female jumper cable | The plug fits the Cardputer's Grove port. |
| jumper wires | female to female, or a STEMMA QT to female jumper cable (Adafruit 4397) for the breakout | |
| USB-C cable | one that carries data | To flash the firmware and send it transactions. |

## Shopping list

For one signer, checked 2026-10-14. Stock changes, so check again the week you order.

| how many | what |
|---|---|
| 1 | M5Stack Cardputer v1.1, SKU K132-V11 |
| 2 | ATECC608 breakouts, such as Adafruit 4314. One is the spare for the [key ceremony](../docs/CEREMONY.md). |
| 1 | AMS1117-3.3 regulator module |
| 1 | Grove to female jumper cable |
| 1 | STEMMA QT to female jumper cable (Adafruit 4397), or female to female jumper wires |
| 1 | USB-C cable that carries data |
| | a multimeter, for the checks below |

- **The Cardputer v1.1 is going away.** Shops list it as discontinued (Switch Science, The Pi Hut,
  2026-10), with the Cardputer-Adv (K132-Adv) as its successor. The Adv reads its keyboard through
  a TCA8418 chip over I2C, on G8, G9 and G11. This firmware drives those three pins as outputs, the
  v1.1's keyboard select lines. **Don't flash it on an Adv.** M5Stack's docs show the same screen
  and Grove pins on both, but no Adv has been on the bench.
- **The chip must be blank**, with both zones open, as Adafruit's 4314 boards have come so far.
  Microchip's TrustFLEX and Trust&GO parts (ATECC608B-TFLXTLS, ATECC608B-TNGTLS) come locked with
  Microchip's own rules, and the ceremony refuses them.
- **608A, 608B or 608C.** Adafruit's page doesn't say which part the 4314 carries. The idle screen
  names it from its revision: `00006002` is a 608A, `00006003` a 608B, `00006005` a 608C. The
  ceremony has run on 608As only.

## Why a regulator

The Grove port gives G2, G1, GND and **5 V**. The ESP32-S3's pins take 3.3 V at most. An ATECC608
run from 5 V would also want its I2C lines pulled up to 5 V, and that would put 5 V on G1 and G2.
So the regulator makes 3.3 V from the Grove 5 V, and the chip and its pull-ups run from that.

## Wiring

| from | to |
|---|---|
| Grove 5 V | regulator IN |
| Grove GND | regulator GND, and the breakout's GND |
| regulator OUT (3.3 V) | the breakout's VIN |
| Grove G2 | the breakout's SDA |
| Grove G1 | the breakout's SCL |

**Match the wires by the pin, not by colour.** On M5Stack's own Grove cables, red is 5 V, black is
GND, yellow is G2 and white is G1. Other Grove cables may differ. A STEMMA QT cable uses yellow for
SCL and blue for SDA, so joining yellow to yellow is wrong. Swapping SDA and SCL does no harm: the
chip just won't answer.

Your regulator module's pins may be labelled VIN/IN, GND and VOUT/OUT.

## Putting it together

1. Plug the Grove cable into the Cardputer and connect the regulator, but not the chip yet.
2. Turn the Cardputer on. With a meter, check about 5 V between the regulator's IN and GND, and
   about 3.3 V between OUT and GND.
3. Turn it off. Connect the breakout: VIN to the regulator's OUT, GND, SDA to G2, SCL to G1.
4. Flash the firmware ([the README](../README.md#flash-it)). The idle screen shows the chip's model
   and serial number. If it says "No chip: review only", check the wiring and press Enter to read
   the chip again.

The chip has power whenever the Cardputer is on. The Grove port has only two signal pins, so the
firmware can't switch the chip's power. It finds the chip by asking for it at 0x60.

## key-c: the key on a USB-C board

key-c is a 16 × 36 mm board with an ATECC608C behind a USB-C socket, in place of the breakout. I had
five made and assembled at JLCPCB in 2026-10; the design files aren't in this repository. The socket
is only wires, not USB:

| key-c's socket | is |
|---|---|
| VBUS | the chip's power |
| D+ | SDA |
| D− | SCL |
| CC1, CC2 | 5.1 kΩ to GND each |
| GND | GND |

Both rows of the socket are wired, so the plug goes in either way up. An ESD array sits between the
socket and the chip. **The board has no pull-ups on SDA and SCL.** Adafruit's breakout has them;
here they go at the Cardputer's end.

### Its end on the Cardputer

A USB-C **plug** breakout (a male plug on a small board, with VBUS, D+, D−, CC and GND brought out)
and two 2.2 kΩ resistors, on the same AMS1117 as before:

| from | to |
|---|---|
| Grove 5 V | regulator IN |
| Grove GND | regulator GND, and the plug's GND |
| regulator OUT (3.3 V) | the plug's VBUS, and a 2.2 kΩ to each of SDA and SCL |
| Grove G2 | the plug's D+ (SDA) |
| Grove G1 | the plug's D− (SCL) |
| the plug's CC | not connected: the Grove port has no pin to spare |

- **A plug, not a socket and a cable.** With a socket on the Cardputer's end, someone will plug a
  phone or a charger into it, and a charger puts 5 V on VBUS. That would reach the 3.3 V line, and
  G1 and G2 through the pull-ups. A plug only takes a key. If you do use a cable, keep it short
  (30 cm or less): a cable's capacitance slows the I2C edges.
- **Nothing on the Cardputer sees the key come and go.** The firmware finds it by asking at 0x60,
  as with the breakout. The ceremony and the signer read its serial again before anything that
  changes the chip or signs ([the ceremony](../docs/CEREMONY.md#the-decisions), decision 10).
- **Don't put a key into a computer, phone or charger.** A port that sees its CC resistors gives it
  5 V. The ATECC608 runs on 2.0 to 5.5 V, so that shouldn't harm it, but nothing there talks to
  it. It's a key on wires, not a USB device.

### Checks, before a key's first plug-in

On each board, unpowered, with a loupe and a meter:
1. Look it over. Sand the edges lightly: they weren't deburred.
2. VBUS to GND (at the socket, or the test pads on the back): not a short.
3. Each CC pin to GND: 5.1 kΩ.
4. SDA and SCL: not shorted to each other, to GND or to VBUS.

On the Cardputer's end, powered, with no key in:
1. The plug's VBUS to GND: about 3.3 V.
2. D+ and D− to GND: about 3.3 V each, through the pull-ups.

Then, with the Cardputer off, plug a key in. Turn it on: the idle screen should name an ATECC608C,
its serial, and counter 0. If it says "No chip", turn it off and check the wiring; Enter reads the
chip again. Only one key at a time; there is only one plug.

## About the chip

The firmware only reads the chip: its serial number, revision, lock state and a counter. It has no
command that writes, locks or makes a key. An ATECC608's locks are permanent, and a chip locked with
the wrong configuration can't be fixed. Signing (milestone 3) will need a key made inside the chip.
How, and in what order, is the [key ceremony](../docs/CEREMONY.md). It is a plan: no firmware runs
it yet.

## A case

Not yet. I plan to add a 3D-printable case here for the regulator and the chip.
