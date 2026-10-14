# Instructions for AI coding agents

diykey: a hardware signer on an M5Stack Cardputer with an ATECC608, and the same code as an
emulator at diykey.app with a passkey as its key. The README says how the pieces fit; read it first.
`hardware/` has the parts and the wiring, `docs/PASSKEY.md` the passkey, `docs/BUILDLOG.md` what
ran on the bench.
Do what was asked and no more. Keep changes small and plain, like the code around them. Don't add a
crate, a network path or a new way to sign without asking.

## Never weaken these

1. **The device signs only the hash it worked out**, from the fields it showed. Never a hash or
   digest a computer, a page or a service hands it. Every page is seen and Sign is held before it
   signs; Reject stays one press. The emulator (`wasm/`) keeps the same rules: it asks the passkey
   to sign only its own hash, from its hash page, and never when the claimed hash differs. The page
   (`web/`) passes on only the hash the emulator asked for.
2. **Keys stay in their chips, and locks are permanent.** Nothing reads a private key out or signs an
   arbitrary digest. `core/src/atecc.rs` holds read commands only. Don't add a command that writes,
   locks, or makes a key (Write, Lock, GenKey with a new key, PrivWrite) unless the person asked for
   exactly that. An ATECC608's locks can't be undone.
3. **Don't flash a board unless asked.** Flashing replaces what's on it.
4. **No secrets in the repository**, nor in tests, logs or screenshots. Testnet only.
   This repository has its own history: don't add another project's remote or push to one.
5. **The supply chain stays pinned.** Every crate exact in `Cargo.toml` (`=x.y.z`), `Cargo.lock`
   committed, builds with `--locked`. Vendored files (`web/vendor/`) are unchanged and their
   checksums written down. Actions are pinned by commit, downloaded tools by SHA-256.
6. **The page loads nothing from anywhere else.** Its Content-Security-Policy allows only its own
   folder (and WebAssembly). The passkey is the emulator's only key: no software key.

## Before you commit

```sh
(cd core && cargo test --locked)
./web/build.sh                     # the screens, the emulator, the firmware image
node web/test.mjs && node web/e2e.mjs
```

Say exactly what ran, what didn't, and what you assumed. Don't call it secure or audited.

## Writing

Short sentences and plain words; say what was checked and how. Keep comments as dense as the code
around them.
