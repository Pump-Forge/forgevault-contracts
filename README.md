# ForgeVault contracts

[ForgeVault](https://vault.pumpforge.tech) locks and vests tokens on **Kaspa L1**: any KCC-20 token
(KasForge, KasSeal, KaspaRocket, standard KRON tokens), and time locks for KRON's own format. A lock is a contract
on the chain. Once tokens are locked, only the contract's rules decide when and to whom they are
released. Nobody, PumpForge included, can release them early or send them anywhere else.

This repository holds the contracts' source and tools to check any lock yourself, so you don't
have to take our word for it. It does not contain ForgeVault's website or service.

> The contracts have **not been independently audited**. See [SECURITY.md](SECURITY.md).

## Contracts

| Contract | Used for | Arguments | Source (sha256) |
|---|---|---|---|
| `ForgeLock v3` | time locks, and each date of a cliff vesting (any KCC-20, and KRON's own format) | `beneficiary` (32-byte x-only key), `unlockDaa` | [forge_lock.sil](contracts/forge_lock.sil) (`c419a9ecf75a8ca609f8d5f3b7c434fe15804806258d9146247c29d15774893f`) |
| `ForgeVest v5` | linear vesting (KCC-20 reference template only; not KRON's own format) | beneficiary, total, start / cliff / end DAA, the token's template, the token id, a salt | [forge_vest.sil](contracts/forge_vest.sil) (`d3cb1c2ee60e801ebb81b4a54a5426a9c5ae43b518206d72fb202ef8bc39afbc`) |
| `ForgeVest v4` | linear vestings made before 27 Oct 2026 (same arguments as v5) | as v5 | [forge_vest_v4.sil](contracts/forge_vest_v4.sil) (`c2f5a4fa4475c9883f8c3914b43b535e2ba5947fb67b8b1dffc2a9709176f7f8`) |

ForgeVest v5 changes three things from v4: `total` must be positive, the claimed token piece must have
borrowing disabled, and the `salt` check can no longer fail (in v4, a salt equal to the token id
would have frozen the vesting).

Compiler: [SilverScript](https://github.com/kaspanet/silverscript) v1.0.0, commit
`3ed973335b59269293564805cc2c58a14595ec03`.

### How a lock holds tokens

A locked token piece is owned by a **script**, not a wallet: its owner is BLAKE2b-256 of the lock
contract (KCC-20 owner scheme 3, pay-to-script-hash; KRON: script owner type 1). The token then
moves only together with a KAS coin locked to the same script, and that coin opens only by the
contract's rules:

- **ForgeLock**: the receiving wallet's signature, once the DAA score reaches `unlockDaa`.
- **ForgeVest**: the receiving wallet takes what has vested by the transaction's DAA score;
  whatever is still locked must stay with the same script.

The chain shows only the hash until the lock is spent. ForgeVault publishes each lock's script and
arguments so anyone can check them.

## Check a lock

ForgeVault's public feed lists every open lock with its holder and contract:
`GET https://vault.pumpforge.tech/api/v1/locked` (`?token=`, `?holder=`, `?beneficiary=`).
Field reference: [vault.pumpforge.tech/docs/api-locked](https://vault.pumpforge.tech/docs/api-locked).

### Full check: rebuild from source (Rust)

```sh
curl -s https://vault.pumpforge.tech/api/v1/locked > locked.json
cd verify/rust && cargo run --release -- ../../locked.json
```

For each lock it:

1. binds `contract.args` to the contract's constructor **by name and type, in source order**
   (anything missing, extra, duplicated, renamed or mistyped fails);
2. compiles the contract in `contracts/` with them and checks the result is
   `contract.redeemScript` byte for byte, and that BLAKE2b-256 of that script is `holder.hash`;
3. checks the rest of the entry against those arguments: `holder.address` is the hash's address,
   `beneficiary` is the script's key, `unlockDaa` and (for ForgeVest) `linear` are the script's
   dates, `token` is the script's `tokenId`, `total` is positive and `amount` is not more than
   `total`.

It prints only values taken from the bound arguments.

### Quick check: no compiler (JavaScript)

```sh
cd verify/js && npm install && node check.mjs            # every open lock
node check.mjs <token id>                                # one token
```

Checks the hash and the holder address, and, for ForgeLock, reads the receiving key and unlock DAA
score straight from the script's fixed layout (minimal number push only) and compares them with the
feed. Other contracts (ForgeVest) are only hash-checked by this tool and print `UNVERIFIED`; use the
Rust check for them. Exit status: 0 all OK, 1 any FAIL, 2 some UNVERIFIED.

```
76043f64dcc7876375768201419d75 ‖ <unlockDaa, minimal script-number push> ‖ 7600050088526a74a569b07620 ‖ <beneficiary, 32 bytes> ‖ ac697551676a68
```

### The last step: the chain

Both tools prove that the script and its hash match. To finish, confirm on the chain that the
token piece really is owned by `holder.hash`: open the token on [Kascov](https://kascov.io) and
find `holder.address` among its holders, or look up `tokensOutpoint` on your own node.

## Known limits

These hold for every lock ForgeVault builds; they matter if you build a lock some other way.

- **ForgeVest keeps `total` on the schedule, not the piece's whole balance.** ForgeVault always
  locks exactly `total` tokens. Tokens in a piece beyond `total` would not be protected; the Rust
  check fails such an entry. (v5 also refuses `total <= 0`; on v4 such a script locks nothing,
  and the Rust check fails it too.)
- **ForgeVest needs the KCC-20 reference template's state layout** (template hash first, owner
  scheme 3). It is never used for KRON's own format: those tokens are locked with ForgeLock dates.
- **The lock coin and the token piece are separate outputs.** After the unlock date the
  beneficiary could spend the KAS lock coin alone (only with a hand-built transaction; wallets do
  not show it). The tokens stay owned by the script and are recovered by sending a new coin to the
  same script address.
- **Borrowing is disabled on every locked piece** when ForgeVault creates it. ForgeVest requires
  it on the remainder after each claim, and v5 also on the piece being claimed.
- **ForgeVest v4 only: `salt` must differ from `tokenId`.** ForgeVault derives the salt from the
  locking transaction's first input, never from the token id. v5 has no such condition.

## Dates

Kaspa contracts count time in DAA score, which rises about ten times a second. The dates
ForgeVault shows are estimates; the DAA score is the rule.

## License

Source-available for verification only: see [LICENSE](LICENSE). You may read, compile and run
these files to check ForgeVault locks; you may not deploy these contracts in your own product or
service without permission.
