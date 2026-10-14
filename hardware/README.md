# The hardware

Three parts and some wires: a Cardputer, a 3.3 V regulator and an ATECC608 breakout on the Grove
port. No soldering if your regulator module and breakout come with header pins fitted.

![Wiring: Grove 5 V through an AMS1117-3.3 to the ATECC608's VIN; G2 to SDA, G1 to SCL](wiring.svg)

## Parts

| part | what I used | notes |
|---|---|---|
| M5Stack Cardputer v1.1 | the v1.1 (Stamp-S3A: ESP32-S3, 8 MB flash, no PSRAM) | The pins in the firmware were checked on a v1.1 only. |
| ATECC608 breakout | an ATECC608A on a breakout board, such as Adafruit's STEMMA QT board (4314) | Any ATECC608A or 608B at I2C address 0x60. The board should have its own SDA and SCL pull-ups. Adafruit's has them. |
| 3.3 V regulator | HiLetgo AMS1117-3.3 module (a pack of 10) | Sold as a "DC-DC step-down buck module"; it's really an AMS1117 linear regulator, up to 800 mA. Grove's 5 V in, 3.3 V out to the chip. |
| Grove cable | Grove (HY2.0-4P) to female jumper cable | The plug fits the Cardputer's Grove port. |
| jumper wires | female to female, or a STEMMA QT to female jumper cable (Adafruit 4397) for the breakout | |
| USB-C cable | one that carries data | To flash the firmware and send it transactions. |

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

## About the chip

The firmware only reads the chip: its serial number, revision, lock state and a counter. It has no
command that writes, locks or makes a key. An ATECC608's locks are permanent, and a chip locked with
the wrong configuration can't be fixed. Signing (milestone 3) will need a key made inside the chip;
how to set that up will be written here first.

## A case

Not yet. I plan to add a 3D-printable case here for the regulator and the chip.
