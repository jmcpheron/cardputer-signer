# Where the vectors come from

They come from outside this Rust code, so the tests check it against something it didn't produce.

- `safe_tx.json`: carried over from the MicroPython version this signer was ported from. Each hash
  was made with viem's `hashTypedData`. The `real-*` ones are Ethereum mainnet transactions, with
  their hashes from Safe's transaction service. Each `expect` is what the MicroPython decoder says
  about that transaction (summary, items, levels). Only the `description` texts were edited, to
  drop names of files that aren't in this repository; no field or hash changed.
- `blockies.json`: drawn by the MicroPython `blockies.py` (`build()` and `hsl_to_rgb()`) under
  CPython. That file started in Austin Griffith's picowallet. It is a port of
  github.com/ethereum/blockies and draws the same pictures as the Safe app.
