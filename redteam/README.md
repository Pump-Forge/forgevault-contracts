# kaspa-redteam run on ForgeVault (30 October 2026)

We ran ForgeVault's mainnet contracts through [kaspa-redteam](https://github.com/kasperolabs/kaspa-redteam), the open-source covenant attack tool by Kaspero Labs. We ran it ourselves, on our own machine. This is a self-run test, not an audit, and Kaspero Labs did not review ForgeVault.

- Contracts: `contracts/forge_lock.sil` (ForgeLock v4) and `contracts/forge_vest.sil` (ForgeVest v6) in this repo.
- Tool: kaspa-redteam v0.4.9 (library v0.3.2, commit `ba0df83`).
- Judge: the rusty-kaspa 2.1.0 consensus validator, so every attack transaction is accepted or rejected by real consensus rules.

## Result: 0 real findings

| Run | What the attacker holds | Result |
|---|---|---|
| `search-forgelock.txt` | nothing (outside key) | 0 findings: an invalid signature and the attacker's own key are both rejected |
| `forgelock-outsider.txt`, `forgevest-outsider.txt` | nothing; real ForgeVault unlocks and claims attacked as given | 0 findings: every path needs the beneficiary's SIGHASH_ALL signature, so no outside attack can even be built |
| `forgelock-beneficiary.txt`, `forgevest-beneficiary.txt` | **the beneficiary's own key** | 12 flags in all, every one the beneficiary moving KAS that is already theirs (see below) |
| `search-forgevest.txt` | nothing | 0 findings; the search can't build a vest claim without a real token, so the baseline runs above cover ForgeVest |

### About the 12 flags in the beneficiary runs

In these runs we gave the tool the beneficiary's key, so the "attacker" is the beneficiary. The tool says so in every flag: *"the attacker signs with the baseline's listed key … an outsider cannot"*. Its own docs say that listing a key that is allowed to spend produces exactly these flags. Each one moves only the beneficiary's own KAS:

- **OUT-02 / OUT-03 / OUT-04:** the small KAS deposit that rides on the beneficiary's own token piece (or on their remaining vesting piece) is lowered. That KAS belongs to the beneficiary.
- **FEE-01:** the beneficiary pays a larger network fee from their own KAS.

In every run, the token amounts, the unlock date, the vesting schedule and the recipient stayed enforced, and no attack sent anything to someone other than the beneficiary.

## Reproduce

```
git clone https://github.com/kasperolabs/kaspa-redteam
cd kaspa-redteam
git checkout ba0df83
cargo build --release
```

Then, from this repo's root (`RT` = path to the built `kaspa-redteam` binary):

```
RT -v contracts/forge_lock.sil --arg beneficiary=@party --arg unlockDaa=100
RT -v contracts/forge_vest.sil --arg beneficiary=@party
RT -v --baseline redteam/baselines/forgelock-outsider.json
RT -v --baseline redteam/baselines/forgevest-outsider.json
RT -v --baseline redteam/baselines/forgelock-beneficiary.json
RT -v --baseline redteam/baselines/forgevest-beneficiary.json
RT replay redteam/findings/forgevest-beneficiary/OUT-02.json
```

- `baselines/`: real ForgeLock unlocks (on the date, after it) and ForgeVest claims (just after the cliff, half way, final), built by ForgeVault's own transaction builder in the tool's baseline format. The `-beneficiary` files list a test-only key (32 bytes of `0x3e`); it holds nothing on any network.
- `reports/`: the tool's output for each run.
- `findings/`: the flagged transactions, replayable with `replay`.
