"""Send a Safe transaction to the Cardputer signer over USB, and print what it says back.

    uvx --with pyserial==3.5 python host/send.py eth-send-base-sepolia      # a vector, by name
    uvx --with pyserial==3.5 python host/send.py --tamper erc20-transfer    # claim the wrong hash
    uvx --with pyserial==3.5 python host/send.py --json tx.json             # your own transaction
    uvx --with pyserial==3.5 python host/send.py --list

A transaction is one line of text (core/src/proto.rs). The hash in `expect` is only compared: the
device works out its own and never signs the one it is sent. A JSON file is shaped like an entry of
vectors/safe_tx.json: chainId, safe, tx {to, value, data, operation, nonce, ...}, and optionally
safeTxHash.
"""
import argparse
import glob
import json
import os
import sys
import time

import serial

HERE = os.path.dirname(os.path.abspath(__file__))
VECTORS = os.path.join(HERE, "..", "vectors", "safe_tx.json")
ZERO = "0x0000000000000000000000000000000000000000"


def line_for(v, tamper=False):
    t = v["tx"]
    f = [("chain", v["chainId"]), ("safe", v["safe"]), ("to", t["to"]), ("value", t.get("value", "0")),
         ("data", t.get("data") or "0x"), ("op", t.get("operation", 0)), ("nonce", t["nonce"]),
         ("safetxgas", t.get("safeTxGas", "0")), ("basegas", t.get("baseGas", "0")),
         ("gasprice", t.get("gasPrice", "0")), ("gastoken", t.get("gasToken") or ZERO),
         ("refundreceiver", t.get("refundReceiver") or ZERO)]
    expect = v.get("safeTxHash")
    if expect and tamper:
        expect = expect[:-1] + ("1" if expect[-1] == "0" else "0")
    if expect:
        f.append(("expect", expect))
    return "tx " + " ".join("%s=%s" % (k, val) for k, val in f)


def port():
    ports = sorted(glob.glob("/dev/cu.usbmodem*") + glob.glob("/dev/ttyACM*"))
    if not ports:
        sys.exit("no board on USB (looked for /dev/cu.usbmodem* and /dev/ttyACM*)")
    return ports[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("name", nargs="?", help="a vector in vectors/safe_tx.json")
    ap.add_argument("--json", help="a transaction in a JSON file instead")
    ap.add_argument("--tamper", action="store_true", help="claim a hash one digit off, to see the device refuse it")
    ap.add_argument("--list", action="store_true", help="list the vectors")
    ap.add_argument("--port", help="the serial port (default: the first /dev/cu.usbmodem*)")
    ap.add_argument("--wait", type=float, default=300, help="seconds to wait for a decision (default 300)")
    a = ap.parse_args()

    vectors = json.load(open(VECTORS))["vectors"]
    if a.list:
        for v in vectors:
            print("%-24s %s" % (v["name"], v.get("description", "")))
        return
    if a.json:
        v = json.load(open(a.json))
    elif a.name:
        found = [v for v in vectors if v["name"] == a.name]
        if not found:
            sys.exit("no vector %r (--list shows them)" % a.name)
        v = found[0]
    else:
        ap.error("name a vector, or --json FILE")

    msg = line_for(v, a.tamper)
    s = serial.Serial(a.port or port(), 115200, timeout=0.2)
    time.sleep(0.2)
    s.reset_input_buffer()
    print(">", msg if len(msg) < 200 else msg[:200] + "…")
    s.write(msg.encode() + b"\n")
    s.flush()

    end, buf = time.time() + a.wait, b""
    while time.time() < end:
        buf += s.read(512)
        while b"\n" in buf:
            raw, buf = buf.split(b"\n", 1)
            text = raw.decode("utf-8", "replace").strip()
            if not text:
                continue
            if not text.startswith("= "):
                print("  " + text)         # the device's log
                continue
            print("<", text)
            word = text[2:].split()[0]
            if word == "hash" and len(text.split()[-1]) != 66:
                print("  INCOMPLETE: that hash isn't 32 bytes; the USB link dropped some")
            elif word == "hash" and v.get("safeTxHash"):
                same = text.split()[2].lower() == v["safeTxHash"].lower()
                print("  " + ("matches the vector's safeTxHash" if same else "DIFFERENT from the vector's safeTxHash"))
                print("  on the device now: page through it, Del rejects")
            if word in ("rejected", "busy", "error", "signed"):
                return
    print("no decision in %d s" % a.wait)


if __name__ == "__main__":
    main()
