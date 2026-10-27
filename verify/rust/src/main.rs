//! forgevault-verify: check ForgeVault locks against the published contract sources.
//!
//!   curl -s https://vault.pumpforge.tech/api/v1/locked > locked.json
//!   cargo run --release -- locked.json
//!
//! For every lock in the feed it compiles the contract source in `../../contracts` with the
//! lock's arguments (`contract.args`), checks the result is `contract.redeemScript` byte for
//! byte, and checks BLAKE2b-256 of that script is `holder.hash`: the owner of the token piece on
//! chain (Kaspa pay-to-script-hash). If all three agree, the rules printed are the rules the chain
//! enforces for those tokens.
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use silverscript_abi::ArtifactValue;
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

const LOCK_SRC: &str = include_str!("../../../contracts/forge_lock.sil");
const VEST_SRC: &str = include_str!("../../../contracts/forge_vest.sil");

fn unhex(s: &str) -> Result<Vec<u8>> {
    let mut out = vec![0u8; s.len() / 2];
    faster_hex::hex_decode(s.as_bytes(), &mut out).map_err(|e| anyhow!("hex: {e}"))?;
    Ok(out)
}
/// The contract compiled with the given arguments (as the feed lists them).
pub fn compile(name: &str, args: &[Value]) -> Result<Vec<u8>> {
    let src = match name {
        "ForgeLock v3" => LOCK_SRC,
        "ForgeVest v4" => VEST_SRC,
        other => bail!("unknown contract {other}"),
    };
    let vals = args
        .iter()
        .map(|a| match a["type"].as_str() {
            Some("int") => a["value"].as_i64().map(ArtifactValue::Int).ok_or_else(|| anyhow!("{}: not an integer", a["name"])),
            Some(_) => unhex(a["value"].as_str().unwrap_or("")).map(ArtifactValue::Bytes),
            None => bail!("argument without a type"),
        })
        .collect::<Result<Vec<_>>>()?;
    let art = compile_to_sil_abi_artifact_with_options(src, &vals, CompileOptions::default()).map_err(|e| anyhow!("compile: {e}"))?;
    let (_, c) = art.contracts.first_key_value().ok_or_else(|| anyhow!("no contract"))?;
    Ok(c.compiled.bytecode.clone())
}
/// BLAKE2b-256: the hash a Kaspa pay-to-script-hash output commits to.
pub fn script_hash(script: &[u8]) -> Vec<u8> {
    blake2b_simd::Params::new().hash_length(32).hash(script).as_bytes().to_vec()
}
/// Check one lock from the feed: Ok(description) or the reason it does not hold.
pub fn check(l: &Value) -> Result<String> {
    let name = l["contract"]["name"].as_str().unwrap_or("");
    let args = l["contract"]["args"].as_array().ok_or_else(|| anyhow!("no contract.args"))?;
    let built = compile(name, args)?;
    let given = unhex(l["contract"]["redeemScript"].as_str().unwrap_or(""))?;
    if built != given {
        bail!("the published {name} source with these arguments does not give contract.redeemScript");
    }
    let hash = faster_hex::hex_string(&script_hash(&built));
    if l["holder"]["hash"].as_str() != Some(hash.as_str()) {
        bail!("BLAKE2b-256(script) {hash} is not holder.hash");
    }
    let arg = |n: &str| args.iter().find(|a| a["name"] == n).map(|a| a["value"].as_str().map(str::to_string).unwrap_or_else(|| a["value"].to_string())).unwrap_or_default();
    Ok(match name {
        "ForgeVest v4" => format!(
            "{name}: {} tokens of {} vest to key {} from DAA {} (cliff {}) to {}",
            arg("total"), l["token"].as_str().unwrap_or("?"), arg("beneficiary"), arg("startDaa"), arg("cliffDaa"), arg("endDaa")
        ),
        _ => format!("{name}: {} tokens of {} unlock to key {} at DAA {}", l["amount"].as_str().unwrap_or("?"), l["token"].as_str().unwrap_or("?"), arg("beneficiary"), arg("unlockDaa")),
    })
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "-".into());
    let text = if path == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s
    } else {
        std::fs::read_to_string(&path).with_context(|| format!("read {path}"))?
    };
    let feed: Value = serde_json::from_str(&text).context("the feed is not JSON")?;
    let locks = feed["locks"].as_array().ok_or_else(|| anyhow!("no locks in the feed"))?;
    let mut bad = 0;
    for l in locks {
        let id = l["id"].as_str().unwrap_or("?");
        match check(l) {
            Ok(d) => println!("OK    {id}  holder {}\n      {d}", l["holder"]["address"].as_str().unwrap_or("")),
            Err(e) => {
                bad += 1;
                println!("FAIL  {id}  {e}");
            }
        }
    }
    println!("\n{} locks checked, {} failed", locks.len(), bad);
    if bad > 0 {
        std::process::exit(1);
    }
    Ok(())
}
