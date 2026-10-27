// Quick check of ForgeVault locks, no compiler needed:
//   npm install && node check.mjs                    (all open locks on mainnet)
//   node check.mjs <token id>                        (one token)
//   node check.mjs --file locked.json                (a saved copy of the feed)
// For every lock: BLAKE2b-256(redeemScript) must be holder.hash (the token piece's owner on
// chain). For ForgeLock it also reads the receiving key and unlock DAA score out of the script's
// fixed layout and compares them with the feed. For a byte-for-byte rebuild of every contract
// from source (ForgeVest included), use ../rust.
import { blake2b } from '@noble/hashes/blake2b';
import fs from 'node:fs';

const FEED = 'https://vault.pumpforge.tech/api/v1/locked';
// ForgeLock v3 = PRE ‖ push(unlockDaa) ‖ MID ‖ beneficiary(32) ‖ POST
const PRE = '76043f64dcc7876375768201419d75', MID = '7600050088526a74a569b07620', POST = 'ac697551676a68';

const hex = (u8) => Buffer.from(u8).toString('hex');
const hash = (scriptHex) => hex(blake2b(Buffer.from(scriptHex, 'hex'), { dkLen: 32 }));
/** Reads a minimal script-number push at the start of `h`: [value, hex length] or null. */
function readInt(h) {
  const op = parseInt(h.slice(0, 2), 16);
  if (op === 0) return [0n, 2];
  if (op >= 0x51 && op <= 0x60) return [BigInt(op - 0x50), 2];
  if (op < 1 || op > 8) return null;
  const b = Buffer.from(h.slice(2, 2 + op * 2), 'hex');
  let v = 0n;
  for (let i = b.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(i === b.length - 1 ? b[i] & 0x7f : b[i]);
  return [b[b.length - 1] & 0x80 ? -v : v, 2 + op * 2];
}
function readForgeLock(s) {
  if (!s.startsWith(PRE) || !s.endsWith(POST)) return null;
  const n = readInt(s.slice(PRE.length));
  if (!n) return null;
  const rest = s.slice(PRE.length + n[1]);
  if (!rest.startsWith(MID) || rest.length !== MID.length + 64 + POST.length) return null;
  return { unlockDaa: n[0], beneficiary: rest.slice(MID.length, MID.length + 64) };
}

const args = process.argv.slice(2);
const feed = args[0] === '--file'
  ? JSON.parse(fs.readFileSync(args[1], 'utf8'))
  : await (await fetch(FEED + (args[0] ? '?token=' + encodeURIComponent(args[0]) : ''))).json();
let bad = 0;
for (const l of feed.locks || []) {
  const s = String(l.contract.redeemScript).toLowerCase(), problems = [];
  if (hash(s) !== l.holder.hash) problems.push('BLAKE2b-256(redeemScript) is not holder.hash');
  let what = '';
  if (l.contract.name === 'ForgeLock v3') {
    const f = readForgeLock(s);
    const arg = (n) => (l.contract.args || []).find((a) => a.name === n)?.value;
    if (!f) problems.push('not the ForgeLock v3 layout');
    else {
      if (f.beneficiary !== arg('beneficiary')) problems.push('the script releases to another key than the feed says');
      if (String(f.unlockDaa) !== String(arg('unlockDaa'))) problems.push('the script unlocks at another DAA score than the feed says');
      what = `${l.amount} tokens unlock to key ${f.beneficiary} at DAA ${f.unlockDaa}`;
    }
  } else what = `${l.contract.name}: hash checked; rebuild it from source with ../rust`;
  if (problems.length) { bad++; console.log(`FAIL  ${l.id}  ${problems.join('; ')}`); }
  else console.log(`OK    ${l.id}  holder ${l.holder.address}\n      ${what}`);
}
console.log(`\n${(feed.locks || []).length} locks checked, ${bad} failed`);
process.exit(bad ? 1 : 0);
