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
| `ForgeLock v4` | time locks, and each date of a cliff vesting (any KCC-20, and KRON's own format) | `beneficiary` (32-byte x-only key), `unlockDaa` | [forge_lock.sil](contracts/forge_lock.sil) (`c4742b7d27aeb277adf6cded5b05dec6e8fb2d0d539bd67d766114e13be19768`) |
| `ForgeVest v6` | linear vesting (KCC-20 reference template only; not KRON's own format) | beneficiary, total, start / cliff / end DAA, the token's template, the token id, a salt | [forge_vest.sil](contracts/forge_vest.sil) (`cb008c030d11b9b2cb5a763181b7c3b6f2504d24070682f9fb165f7e7c0530b5`) |

Both accept only a `SIGHASH_ALL` signature, so a signature always covers the whole transaction,
including where the tokens go. [contracts/legacy](contracts/legacy) holds the earlier versions used
by test locks made before 27 Oct 2026, so the checkers can still check those.

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
   `beneficiary` is the script's key, and `unlockDaa` and (for ForgeVest) `linear` are the
   script's dates;
4. for ForgeVest, checks the vesting can be claimed at all: `token` is the script's `tokenId`,
   `total` is positive, `startDaa >= 0`, `endDaa - startDaa` is at most `MAX_SPAN` (about 2000
   days), and the template hash is a known KCC-20 reference build with matching `tplPrefixLen` /
   `tplSuffixLen` (`verify/rust/src/templates.rs`);
5. fails an entry whose `amount` is not positive. An empty feed prints that nothing was checked.

The values it checks are printed from the bound arguments. A ForgeLock names no token and no
amount, and a ForgeVest's current balance is not in its script: those are printed as
`feed only (not checked)`. This tool does not read the chain (see the last step below).

### Quick check: no compiler (JavaScript)

```sh
cd verify/js && npm install && node check.mjs            # every open lock
node check.mjs <token id>                                # one token
```

Checks the hash and the holder address, and, for ForgeLock, reads the receiving key and unlock DAA
score straight from the script's fixed layout (minimal number push only) and compares them with the
feed (token and amount are printed as feed only). Other contracts (ForgeVest) are only
hash-checked by this tool and print `UNVERIFIED`; use the Rust check for them. Exit status: 0 all
OK, 1 any FAIL, 2 some UNVERIFIED.

```
ForgeLock v4: 76043f64dcc7876375768201419d75 ‖ <unlockDaa, minimal script-number push> ‖ 7600050088526a74a569b07676827c75519601419c6976014001417f010187697820 ‖ <beneficiary, 32 bytes> ‖ ac69757551676a68
```

### The last step: the chain

Both tools prove that the script and its hash match. To finish, check the token piece at
`tokensOutpoint` on the chain (on [Kascov](https://kascov.io), find `holder.address` among the
token's holders, or look it up on your own node):

- its owner is `holder.hash` with owner scheme 3 (pay-to-script-hash; KRON: script owner type 1);
- its borrow scheme is 0 (borrowing disabled; ForgeVault always creates locked pieces this way);
- it holds the amount the feed shows.

## Known limits

These hold for every lock ForgeVault builds; they matter only for a lock built some other way.

- **ForgeVest keeps `total` on the schedule, not the piece's whole balance.** ForgeVault always
  locks exactly `total` tokens. Tokens in a piece beyond `total` are not protected; the Rust check
  fails an entry whose amount is above `total`.
- **ForgeVest's span is capped.** Every claim requires `endDaa - startDaa <= MAX_SPAN`
  (1,728,000,000 DAA score, about 2000 days). A longer vesting could never be claimed; ForgeVault
  never builds one and the Rust check fails it. Time locks have no such cap (ForgeVault allows
  dates up to 10 years away).
- **ForgeVest needs the KCC-20 reference template's state layout** (template hash first, owner
  scheme 3). It is never used for KRON's own format: those tokens are locked with ForgeLock dates.
- **Borrowing must be disabled on the locked piece.** ForgeVault always creates it that way, and
  ForgeVest requires it on the piece it releases and on the remainder. A piece locked with
  borrowing on could not be claimed from a ForgeVest.
- **ForgeLock: the lock coin and the token piece are separate outputs.** After the unlock date the
  beneficiary could spend the KAS lock coin alone (only with a hand-built transaction; wallets do
  not show it). The tokens stay owned by the script and are recovered by sending a new coin to the
  same script address. A ForgeVest lock coin cannot be spent without the token piece.
- **Don't send KAS to a lock's address by hand.** The site adds the lock coin itself. Extra KAS
  sent to a ForgeVest address stays there once its tokens are fully claimed.

## Dates

Kaspa contracts count time in DAA score, which rises about ten times a second. The dates
ForgeVault shows are estimates; the DAA score is the rule.

## License

Source-available for verification only: see [LICENSE](LICENSE). You may read, compile and run
these files to check ForgeVault locks; you may not deploy these contracts in your own product or
service without permission.
