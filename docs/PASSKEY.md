# A passkey as the key

At [diykey.app](https://diykey.app), the emulator runs the signer's own code (`core`, compiled to
WebAssembly) and uses your passkey as its key, in place of the ATECC608. This page says how close
that is to the chip, where it isn't, and the message both of them sign.

## How a passkey compares with the ATECC608

| | ATECC608 on the Cardputer | a passkey in the emulator |
|---|---|---|
| curve | P-256 | P-256 (WebAuthn's ES256) |
| where the key is made | inside the chip | inside the authenticator: the Secure Enclave, a TPM, Android's keystore, or a security key |
| reading it out | the chip has no command for it | WebAuthn has no way to |
| copies | none | **synced passkeys** (iCloud Keychain, Google Password Manager) are copied, end-to-end encrypted, to your other devices. A security key's passkey isn't. The page shows the BS flag when a passkey is synced. |
| what you check before signing | the Cardputer's own screen | the page's emulator screen, drawn by your browser. The passkey's own prompt shows the site's name, not the transaction. |
| how you approve | hold Sign (milestone 3) | Face ID, Touch ID, Windows Hello or a PIN: user verification (UV) |
| what it signs | the hash the Cardputer worked out | the hash the emulator worked out, as the WebAuthn challenge |

The biggest difference is the screen. On the Cardputer, what you check and what signs are on a device
your computer can't change. In the emulator, both are in the browser. A compromised computer could
show one transaction and ask the passkey to sign another, and the passkey's prompt wouldn't tell you.

So the emulator is for trying the signer, and a passkey is a reasonable Safe owner next to other
kinds of key. Keep passkey owners below a Safe's threshold.

## The address that signs on chain

A P-256 key, the chip's or a passkey's, can't be an Ethereum account by itself. Safe gives each
one its own small contract, a `SafeWebAuthnSignerProxy` (safe-modules passkey v0.2.1), which Safe's
factory deploys at an address fixed by the key: CREATE2 with salt 0 over the proxy's creation code,
Safe's WebAuthn singleton, the key's x and y, and the P-256 verifiers. That address is the same on
every chain the factory is on, and it is what a Safe lists as its owner.

The page works it out from the public key (`web/lib/signer.js`) and shows it on the key card: the
passkey's in the emulator, and the chip's over USB, from the public key the firmware reads out of
slot 0. `web/test.mjs` checks it against `vectors/webauthn.json`, whose addresses came from the
factory's `getSigner` on Base Sepolia.

That address only signs. It isn't a wallet: ETH or tokens sent to it can't be moved. To hold funds,
make a Safe with the address as one of its owners, and send them to the Safe.

## A passkey made before

WebAuthn hands out a passkey's public key only when the passkey is made, so the page keeps it in
this browser. To use a passkey made somewhere else (another browser, or synced to your phone),
"Use one I made before" asks it to sign twice. One P-256 signature fits two public keys; the second
signature leaves the one that fits both, and the page checks both signatures with it before using it
(`web/lib/passkey.js`). `web/test.mjs` checks this with keys made in Node, and `web/e2e.mjs` in
Chrome: a passkey forgotten by the page and loaded again gives the same address.

## What the passkey signs

WebAuthn doesn't sign a hash as it is. The emulator asks the passkey to sign with the Safe
transaction hash as the **challenge**. The passkey then signs, with P-256 and SHA-256:

```
authenticatorData ‖ sha256(clientDataJSON)
clientDataJSON = {"type":"webauthn.get","challenge":"<base64url of the hash>","origin":"https://diykey.app",…}
```

That is the message Safe's passkey signer checks (safe-modules' WebAuthn library). It rebuilds
`clientDataJSON` from the Safe transaction hash and the fields after the challenge, so a signature
over any other hash fails there. The signature it reads is:

```
abi.encode(bytes authenticatorData, string clientDataFields, uint256 r, uint256 s)
```

`s` is put in its low form (`s ≤ n/2`), the same signature in the form verifiers accept.
`web/test.mjs` checks this encoding byte for byte against the four signatures in
`vectors/webauthn.json`, which Safe's contract accepted on Base Sepolia (`isValidSignatureForSigner`).

## What the page checks

After each signature, the page checks it with the browser's own crypto (WebCrypto) and shows every
part:

- the challenge is the hash the emulator worked out
- `authenticatorData` is for this site (the SHA-256 of its name), and the origin is this page's
- user present (UP) and user verified (UV)
- `clientDataJSON` starts in the order Safe's signer rebuilds it
- the P-256 signature is valid for the passkey's public key

The emulator asks for a signature only from its hash page, after the pages before it, and only when
the hash this page claimed matches its own. A wrong claim's hash page is red and Enter does nothing.
`node web/test.mjs` checks this with Node as the authenticator, and `node web/e2e.mjs` checks it in
Chrome with Chrome's virtual authenticator.

The page remembers the passkey's credential id and public key in this browser (localStorage), to
check its signatures. "Forget" clears that; the passkey stays in your keychain.

## Not done yet

- **A Safe owner.** No signature is sent anywhere. Making a passkey an owner of a Safe on Base
  Sepolia, through Safe's passkey signer, comes next. The encoding matches signatures the contract
  accepted, but no diykey passkey's signature has been sent to it yet.
- **The Cardputer signing (milestone 3).** The plan: the chip signs this same message. The device
  builds `authenticatorData` and `clientDataJSON` from the hash it worked out, takes their SHA-256,
  and the ATECC608 signs that digest. A chip and a passkey can then own a Safe the same way, through
  Safe's audited passkey signer. The device still signs only its own hash.
