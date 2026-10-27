# ForgeVault contracts

[ForgeVault](https://vault.pumpforge.tech) locks and vests tokens on **Kaspa L1**: any KCC-20 token
(KasForge, KasSeal, KaspaRocket, standard KRON tokens) and KRON's own format. A lock is a contract
on the chain. Once tokens are locked, only the contract's rules decide when and to whom they are
released. Nobody, PumpForge included, can release them early or send them anywhere else.

This repository holds the contracts' source and tools to check any lock yourself, so you don't
have to take our word for it. It does not contain ForgeVault's website or service.

> The contracts have **not been independently audited**. See [SECURITY.md](SECURITY.md).

## Contracts

| Contract | Used for | Arguments | Source (sha256) |
|---|---|---|---|
| `ForgeLock v3` | time locks, and each date of a cliff vesting | `beneficiary` (32-byte x-only key), `unlockDaa` | [forge_lock.sil](contracts/forge_lock.sil) (`c419a9ecf75a8ca609f8d5f3b7c434fe15804806258d9146247c29d15774893f`) |
| `ForgeVest v4` | linear vesting | beneficiary, total, start / cliff / end DAA, the token's template, the token id, a salt | [forge_vest.sil](contracts/forge_vest.sil) (`302585512c09e1d6eba8a2fe754ac4bca9514697ff7b84edd6c4a6fe7c986c19`) |

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

For each lock it compiles the contract in `contracts/` with the lock's arguments, checks the
result is `contract.redeemScript` byte for byte, and checks BLAKE2b-256 of that script is
`holder.hash`.

### Quick check: no compiler (JavaScript)

```sh
cd verify/js && npm install && node check.mjs            # every open lock
node check.mjs <token id>                                # one token
```

Checks the hash and, for ForgeLock, reads the receiving key and unlock DAA score straight from the
script's fixed layout:

```
76043f64dcc7876375768201419d75 ‖ <unlockDaa, minimal script-number push> ‖ 7600050088526a74a569b07620 ‖ <beneficiary, 32 bytes> ‖ ac697551676a68
```

### The last step: the chain

Both tools prove that the script and its hash match. To finish, confirm on the chain that the
token piece really is owned by `holder.hash`: open the token on [Kascov](https://kascov.io) and
find `holder.address` among its holders, or look up `tokensOutpoint` on your own node.

## Dates

Kaspa contracts count time in DAA score, which rises about ten times a second. The dates
ForgeVault shows are estimates; the DAA score is the rule.

## License

Source-available for verification only: see [LICENSE](LICENSE). You may read, compile and run
these files to check ForgeVault locks; you may not deploy these contracts in your own product or
service without permission.
