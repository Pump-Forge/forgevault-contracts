//! forgevault-verify: check ForgeVault locks against the published contract sources.
//!
//!   curl -s https://vault.pumpforge.tech/api/v1/locked > locked.json
//!   cargo run --release -- locked.json
//!
//! For every lock in the feed it binds `contract.args` to the contract's constructor by name and
//! type (in source order, nothing missing, duplicated or extra), compiles the contract source in
//! `../../contracts` with them, checks the result is `contract.redeemScript` byte for byte, and
//! checks BLAKE2b-256 of that script is `holder.hash`: the owner of the token piece on chain
//! (Kaspa pay-to-script-hash). It then checks that what the feed says about the lock is what
//! those arguments say (dates, addresses, and for ForgeVest the token, total and template), and
//! that a ForgeVest can be claimed at all (span, salt, template lengths). Values the script does
//! not contain (a ForgeLock's token and amount, a ForgeVest's current amount) are printed as
//! "feed only": this tool does not read the chain.
use anyhow::{Context, Result, anyhow, bail};
use kaspa_addresses::{Address, Prefix, Version};
use serde_json::Value;
use silverscript_abi::ArtifactValue;
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

mod templates;
const LOCK_SRC: &str = include_str!("../../../contracts/forge_lock.sil");
const LOCK_V3_SRC: &str = include_str!("../../../contracts/legacy/forge_lock_v3.sil");
const VEST_SRC: &str = include_str!("../../../contracts/forge_vest.sil");
const VEST_V4_SRC: &str = include_str!("../../../contracts/legacy/forge_vest_v4.sil");
const VEST_V5_SRC: &str = include_str!("../../../contracts/legacy/forge_vest_v5.sil");
/// ForgeVest's MAX_SPAN: a vesting whose end - start is larger can never be claimed.
const MAX_SPAN: i64 = 1_728_000_000;

/// Each contract's constructor parameters, in source order (`contracts/*.sil`).
const LOCK_PARAMS: &[(&str, &str)] = &[("beneficiary", "pubkey"), ("unlockDaa", "int")];
const VEST_PARAMS: &[(&str, &str)] = &[
    ("beneficiary", "pubkey"),
    ("total", "int"),
    ("startDaa", "int"),
    ("cliffDaa", "int"),
    ("endDaa", "int"),
    ("tplPrefixLen", "int"),
    ("tplSuffixLen", "int"),
    ("tokenTemplateHash", "byte[32]"),
    ("tokenId", "byte[32]"),
    ("salt", "byte[32]"),
];

fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("hex: odd length");
    }
    let mut out = vec![0u8; s.len() / 2];
    faster_hex::hex_decode(s.as_bytes(), &mut out).map_err(|e| anyhow!("hex: {e}"))?;
    Ok(out)
}

/// One bound constructor argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i64),
    Bytes(Vec<u8>),
}
impl Arg {
    fn int(&self) -> i64 {
        match self {
            Arg::Int(v) => *v,
            Arg::Bytes(_) => unreachable!("bound by type"),
        }
    }
    fn bytes(&self) -> &[u8] {
        match self {
            Arg::Bytes(b) => b,
            Arg::Int(_) => unreachable!("bound by type"),
        }
    }
}

/// The contract's source and parameter list.
fn contract(name: &str) -> Result<(&'static str, &'static [(&'static str, &'static str)])> {
    match name {
        "ForgeLock v4" => Ok((LOCK_SRC, LOCK_PARAMS)),
        "ForgeLock v3" => Ok((LOCK_V3_SRC, LOCK_PARAMS)),
        "ForgeVest v6" => Ok((VEST_SRC, VEST_PARAMS)),
        "ForgeVest v5" => Ok((VEST_V5_SRC, VEST_PARAMS)),
        "ForgeVest v4" => Ok((VEST_V4_SRC, VEST_PARAMS)),
        other => bail!("unknown contract {other:?}"),
    }
}

/// Binds the feed's `contract.args` to the constructor: the same names, in source order, with
/// the declared types and sizes. Anything missing, duplicated, extra, renamed or mistyped fails.
pub fn bind(name: &str, args: &[Value]) -> Result<Vec<(&'static str, Arg)>> {
    let (_, params) = contract(name)?;
    if args.len() != params.len() {
        bail!("{name} takes {} arguments, the feed gives {}", params.len(), args.len());
    }
    let mut out = vec![];
    for (i, ((pname, ptype), a)) in params.iter().zip(args).enumerate() {
        let (n, t) = (a["name"].as_str().unwrap_or(""), a["type"].as_str().unwrap_or(""));
        if n != *pname {
            bail!("argument {i} is named {n:?}, the contract's argument {i} is {pname:?}");
        }
        if t != *ptype {
            bail!("{pname} has type {t:?}, the contract declares {ptype:?}");
        }
        let v = match *ptype {
            "int" => Arg::Int(a["value"].as_i64().ok_or_else(|| anyhow!("{pname}: not an integer"))?),
            _ => {
                let b = unhex(a["value"].as_str().ok_or_else(|| anyhow!("{pname}: not a hex string"))?)?;
                if b.len() != 32 {
                    bail!("{pname}: {} bytes, expected 32", b.len());
                }
                Arg::Bytes(b)
            }
        };
        out.push((*pname, v));
    }
    Ok(out)
}

/// The contract compiled with bound arguments.
pub fn compile(name: &str, args: &[(&str, Arg)]) -> Result<Vec<u8>> {
    let (src, _) = contract(name)?;
    let vals: Vec<ArtifactValue> = args
        .iter()
        .map(|(_, a)| match a {
            Arg::Int(v) => ArtifactValue::Int(*v),
            Arg::Bytes(b) => ArtifactValue::Bytes(b.clone()),
        })
        .collect();
    let art = compile_to_sil_abi_artifact_with_options(src, &vals, CompileOptions::default()).map_err(|e| anyhow!("compile: {e}"))?;
    let (_, c) = art.contracts.first_key_value().ok_or_else(|| anyhow!("no contract"))?;
    Ok(c.compiled.bytecode.clone())
}

/// BLAKE2b-256: the hash a Kaspa pay-to-script-hash output commits to.
pub fn script_hash(script: &[u8]) -> Vec<u8> {
    blake2b_simd::Params::new().hash_length(32).hash(script).as_bytes().to_vec()
}

fn hx(b: &[u8]) -> String {
    faster_hex::hex_string(b)
}
/// A feed number given as a JSON number or a decimal string.
fn num(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
fn need(ok: bool, what: impl FnOnce() -> String) -> Result<()> {
    if ok { Ok(()) } else { Err(anyhow!(what())) }
}

/// Check one lock from the feed: Ok(description) or the reason it does not hold.
pub fn check(l: &Value) -> Result<String> {
    let name = l["contract"]["name"].as_str().ok_or_else(|| anyhow!("no contract.name"))?;
    let raw = l["contract"]["args"].as_array().ok_or_else(|| anyhow!("no contract.args"))?;
    let args = bind(name, raw)?;
    let arg = |n: &str| &args.iter().find(|(k, _)| *k == n).expect("bound").1;

    // 1. the script is the published source with these arguments, and the chain's owner hash
    let built = compile(name, &args)?;
    let given = unhex(l["contract"]["redeemScript"].as_str().unwrap_or(""))?;
    need(built == given, || format!("the published {name} source with these arguments does not give contract.redeemScript"))?;
    let hash = script_hash(&built);
    need(l["holder"]["hash"].as_str().map(str::to_ascii_lowercase) == Some(hx(&hash)), || format!("BLAKE2b-256(script) {} is not holder.hash", hx(&hash)))?;

    // 2. the addresses the feed shows are the hash's and the key's
    let holder_addr: Address = l["holder"]["address"].as_str().unwrap_or("").try_into().map_err(|e| anyhow!("holder.address: {e}"))?;
    let prefix: Prefix = holder_addr.prefix;
    need(holder_addr == Address::new(prefix, Version::ScriptHash, &hash), || "holder.address is not the pay-to-script-hash address of holder.hash".into())?;
    let ben = arg("beneficiary").bytes();
    let ben_addr = Address::new(prefix, Version::PubKey, ben).to_string();
    need(l["beneficiary"].as_str() == Some(ben_addr.as_str()), || format!("the feed's beneficiary is not the script's key ({ben_addr})"))?;

    // 3. the lock's terms in the feed are the script's, and the script can be opened
    let amount = num(&l["amount"]).ok_or_else(|| anyhow!("no amount"))?;
    need(amount > 0, || format!("the feed's amount is {amount}: an open lock holds at least one token"))?;
    let token = l["token"].as_str().unwrap_or("?");
    Ok(match name {
        "ForgeVest v6" | "ForgeVest v5" | "ForgeVest v4" => {
            let (total, start, cliff, end) = (arg("total").int(), arg("startDaa").int(), arg("cliffDaa").int(), arg("endDaa").int());
            need(hx(arg("tokenId").bytes()) == token.to_ascii_lowercase(), || format!("the script holds token {}, the feed says {token}", hx(arg("tokenId").bytes())))?;
            need(total > 0, || "total is not positive: nothing would stay locked".into())?;
            need(start >= 0, || "the start is negative".into())?;
            need(start < end && start <= cliff && cliff <= end, || "the schedule is not start <= cliff <= end, start < end".into())?;
            // every claim requires end - start <= MAX_SPAN: a longer vesting can never be claimed
            need(end - start <= MAX_SPAN, || format!("end - start is {}, more than the contract's MAX_SPAN {MAX_SPAN}: it could never be claimed", end - start))?;
            // v4 requires salt != tokenId on every claim: equal ones freeze the vesting
            need(name != "ForgeVest v4" || arg("salt").bytes() != arg("tokenId").bytes(), || "salt equals tokenId: ForgeVest v4 can never be claimed".into())?;
            // the template must be a real KCC-20 build with these state lengths, or every claim fails
            let tpl = hx(arg("tokenTemplateHash").bytes());
            let known = templates::TEMPLATES.iter().find(|t| t.0 == tpl).ok_or_else(|| anyhow!("tokenTemplateHash {tpl} is not a known KCC-20 reference build"))?;
            need(arg("tplPrefixLen").int() == known.1 && arg("tplSuffixLen").int() == known.2, || format!("tplPrefixLen/tplSuffixLen are not those of {} ({}, {})", known.3, known.1, known.2))?;
            // `amount` is the feed's number (this tool does not read the chain); the script keeps
            // at most `total` on the schedule, so a larger number is a lock not fully protected
            need(amount <= total, || format!("the feed says the piece holds {amount} tokens but the script only keeps {total} on the schedule"))?;
            let lin = &l["linear"];
            need(
                num(&lin["total"]) == Some(total) && num(&lin["startDaa"]) == Some(start) && num(&lin["cliffDaa"]) == Some(cliff) && num(&lin["endDaa"]) == Some(end),
                || "the feed's linear schedule is not the script's".into(),
            )?;
            need(num(&l["unlockDaa"]) == Some(end), || "the feed's unlockDaa is not the script's endDaa".into())?;
            format!("{name}: {total} tokens of {token} ({}) vest to {ben_addr} from DAA {start} (cliff {cliff}) to {end}\n      feed only (not checked): {amount} still in the lock", known.3)
        }
        _ => {
            let unlock = arg("unlockDaa").int();
            need(num(&l["unlockDaa"]) == Some(unlock), || "the feed's unlockDaa is not the script's".into())?;
            // a ForgeLock names no token and no amount: those come from the feed only
            format!("{name}: unlocks to {ben_addr} at DAA {unlock}\n      feed only (not checked): {amount} tokens of {token}")
        }
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
    if locks.is_empty() {
        println!("The feed lists no locks: nothing was checked. (An empty feed does not mean a token is locked.)");
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const EVIL: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const TOKEN: &str = "abababababababababababababababababababababababababababababababab";
    // KasForge's build (kcc20 2x3): state prefix 1, suffix 3885
    const TPL: &str = "9703112ee6e3555107cd168858992b77d3b74f655205b2b463b1f9ec2ec73cf7";
    const SALT: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    fn vest_args(ben: &str, salt: &str) -> Value {
        json!([
            { "name": "beneficiary", "type": "pubkey", "value": ben },
            { "name": "total", "type": "int", "value": 1000 },
            { "name": "startDaa", "type": "int", "value": 100 },
            { "name": "cliffDaa", "type": "int", "value": 200 },
            { "name": "endDaa", "type": "int", "value": 1000 },
            { "name": "tplPrefixLen", "type": "int", "value": 1 },
            { "name": "tplSuffixLen", "type": "int", "value": 3885 },
            { "name": "tokenTemplateHash", "type": "byte[32]", "value": TPL },
            { "name": "tokenId", "type": "byte[32]", "value": TOKEN },
            { "name": "salt", "type": "byte[32]", "value": salt },
        ])
    }
    /// A feed entry whose script, hash and addresses are built from `args` exactly as they come.
    fn feed_entry(name: &str, args: Value, ben_shown: &str) -> Value {
        let vals: Vec<ArtifactValue> = args
            .as_array()
            .unwrap()
            .iter()
            .map(|a| match a["type"].as_str().unwrap() {
                "int" => ArtifactValue::Int(a["value"].as_i64().unwrap()),
                _ => ArtifactValue::Bytes(unhex(a["value"].as_str().unwrap()).unwrap()),
            })
            .collect();
        let (src, _) = contract(name).unwrap();
        let art = compile_to_sil_abi_artifact_with_options(src, &vals, CompileOptions::default()).unwrap();
        let script = art.contracts.first_key_value().unwrap().1.compiled.bytecode.clone();
        let h = script_hash(&script);
        json!({
            "id": "t:0", "token": TOKEN, "amount": "1000", "unlockDaa": if name.starts_with("ForgeVest") { 1000 } else { 5000 },
            "linear": { "total": "1000", "startDaa": 100, "cliffDaa": 200, "endDaa": 1000 },
            "beneficiary": Address::new(Prefix::Mainnet, Version::PubKey, &unhex(ben_shown).unwrap()).to_string(),
            "holder": { "address": Address::new(Prefix::Mainnet, Version::ScriptHash, &h).to_string(), "hash": hx(&h) },
            "contract": { "name": name, "redeemScript": hx(&script), "args": args },
        })
    }

    #[test]
    fn honest_entries_pass() {
        for v in ["ForgeVest v6", "ForgeVest v5", "ForgeVest v4"] {
            let ok = check(&feed_entry(v, vest_args(KEY, SALT), KEY)).unwrap();
            assert!(ok.contains("feed only (not checked): 1000 still in the lock"), "{ok}");
        }
        for v in ["ForgeLock v4", "ForgeLock v3"] {
            let lock = json!([{ "name": "beneficiary", "type": "pubkey", "value": KEY }, { "name": "unlockDaa", "type": "int", "value": 5000 }]);
            let ok = check(&feed_entry(v, lock, KEY)).unwrap();
            assert!(ok.contains("feed only (not checked): 1000 tokens of"), "{ok}");
        }
        // v4 and v5 are different scripts: one labelled as the other fails
        let mut e = feed_entry("ForgeVest v4", vest_args(KEY, SALT), KEY);
        e["contract"]["name"] = json!("ForgeVest v5");
        assert!(check(&e).unwrap_err().to_string().contains("redeemScript"));
    }

    /// Vestings that compile and match their hash but can never be claimed fail.
    #[test]
    fn unclaimable_vestings_fail() {
        let with = |name: &str, i: usize, v: Value| {
            let mut a = vest_args(KEY, SALT);
            a[i]["value"] = v;
            let mut e = feed_entry(name, a, KEY);
            e["linear"] = json!({ "total": "1000", "startDaa": e["contract"]["args"][2]["value"], "cliffDaa": e["contract"]["args"][3]["value"], "endDaa": e["contract"]["args"][4]["value"] });
            e["unlockDaa"] = e["contract"]["args"][4]["value"].clone();
            check(&e)
        };
        // a span over MAX_SPAN (six years)
        let e = with("ForgeVest v6", 4, json!(100 + 3_150_000_000i64)).unwrap_err().to_string();
        assert!(e.contains("MAX_SPAN"), "{e}");
        // a wrong template length, an unknown template
        assert!(with("ForgeVest v6", 6, json!(3884)).unwrap_err().to_string().contains("tplSuffixLen"));
        assert!(with("ForgeVest v6", 7, json!("03".repeat(32))).unwrap_err().to_string().contains("not a known"));
        // a negative start
        let mut a = vest_args(KEY, SALT);
        a[2]["value"] = json!(-1);
        let mut e = feed_entry("ForgeVest v5", a, KEY);
        e["linear"]["startDaa"] = json!(-1);
        assert!(check(&e).unwrap_err().to_string().contains("negative"));
        // ForgeVest v4 with salt == tokenId; v5 and v6 have no such condition
        for (v, bad) in [("ForgeVest v4", true), ("ForgeVest v5", false), ("ForgeVest v6", false)] {
            let r = check(&feed_entry(v, vest_args(KEY, TOKEN), KEY));
            assert_eq!(r.is_err(), bad, "{v}: {r:?}");
        }
    }

    #[test]
    fn swapped_names_fail() {
        // the attacker's key compiled first but labelled `salt`, the victim's labelled `beneficiary`
        let mut a = vest_args(EVIL, KEY);
        a[0]["name"] = json!("salt");
        a[9]["name"] = json!("beneficiary");
        let e = check(&feed_entry("ForgeVest v4", a, KEY)).unwrap_err().to_string();
        assert!(e.contains("named"), "{e}");
        // ints swapped by name
        let mut a = vest_args(KEY, SALT);
        a[1]["name"] = json!("endDaa");
        a[4]["name"] = json!("total");
        assert!(check(&feed_entry("ForgeVest v4", a, KEY)).is_err());
    }

    #[test]
    fn feed_must_match_the_script() {
        // another beneficiary shown than the script pays
        assert!(check(&feed_entry("ForgeVest v4", vest_args(EVIL, SALT), KEY)).unwrap_err().to_string().contains("beneficiary"));
        // another token
        let mut e = feed_entry("ForgeVest v5", vest_args(KEY, SALT), KEY);
        e["token"] = json!("cd".repeat(32));
        assert!(check(&e).unwrap_err().to_string().contains("token"));
        // more in the piece than the script keeps on the schedule
        let mut e = feed_entry("ForgeVest v4", vest_args(KEY, SALT), KEY);
        e["amount"] = json!("5000");
        assert!(check(&e).unwrap_err().to_string().contains("only keeps"));
        // a schedule shown that is not the script's
        let mut e = feed_entry("ForgeVest v4", vest_args(KEY, SALT), KEY);
        e["linear"]["endDaa"] = json!(999_999);
        assert!(check(&e).is_err());
        // total <= 0
        let mut a = vest_args(KEY, SALT);
        a[1]["value"] = json!(0);
        let mut e = feed_entry("ForgeVest v4", a, KEY);
        e["amount"] = json!("1");
        e["linear"]["total"] = json!("0");
        assert!(check(&e).unwrap_err().to_string().contains("total"));
        // an amount of 0, and upper-case hex (still the same script)
        let mut e = feed_entry("ForgeVest v6", vest_args(KEY, SALT), KEY);
        e["amount"] = json!("0");
        assert!(check(&e).unwrap_err().to_string().contains("amount"));
        let mut e = feed_entry("ForgeVest v6", vest_args(KEY, SALT), KEY);
        e["holder"]["hash"] = json!(e["holder"]["hash"].as_str().unwrap().to_uppercase());
        e["token"] = json!(TOKEN.to_uppercase());
        check(&e).expect("upper-case hex is the same value");
        // a holder address that is not the hash's
        let mut e = feed_entry("ForgeVest v4", vest_args(KEY, SALT), KEY);
        e["holder"]["address"] = json!(Address::new(Prefix::Mainnet, Version::ScriptHash, &[7u8; 32]).to_string());
        assert!(check(&e).unwrap_err().to_string().contains("holder.address"));
        // missing, extra and mistyped arguments
        let mut a = vest_args(KEY, SALT);
        a.as_array_mut().unwrap().pop();
        assert!(bind("ForgeVest v5", a.as_array().unwrap()).is_err());
        let mut a = vest_args(KEY, SALT);
        a.as_array_mut().unwrap().push(json!({ "name": "x", "type": "int", "value": 1 }));
        assert!(bind("ForgeVest v5", a.as_array().unwrap()).is_err());
        let mut a = vest_args(KEY, SALT);
        a[1]["type"] = json!("byte[32]");
        assert!(bind("ForgeVest v5", a.as_array().unwrap()).is_err());
    }
}
