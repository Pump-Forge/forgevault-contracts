// Quick check of ForgeVault locks, no compiler needed:
//   npm install && node check.mjs                    (all open locks on mainnet)
//   node check.mjs <token id>                        (one token)
//   node check.mjs --file locked.json                (a saved copy of the feed)
// For every lock: BLAKE2b-256(redeemScript) must be holder.hash (the token piece's owner on
// chain), and holder.address must be that hash's address. ForgeLock is checked in full: the
// receiving key and unlock DAA score are read out of the script's fixed layout and compared with
// the feed. Other contracts (ForgeVest) are only hash-checked here and print UNVERIFIED: rebuild
// them from source with ../rust. Exit status: 0 all OK, 1 any FAIL, 2 some UNVERIFIED.
import { blake2b } from '@noble/hashes/blake2b';
import fs from 'node:fs';

const FEED = 'https://vault.pumpforge.tech/api/v1/locked';
// ForgeLock v3 = PRE ‖ push(unlockDaa) ‖ MID ‖ beneficiary(32) ‖ POST
const PRE = '76043f64dcc7876375768201419d75', MID = '7600050088526a74a569b07620', POST = 'ac697551676a68';

const hex = (u8) => Buffer.from(u8).toString('hex');
const hash = (scriptHex) => hex(blake2b(Buffer.from(scriptHex, 'hex'), { dkLen: 32 }));

/** Reads a minimal script-number push at the start of `h`: [value, hex length], or null when
 *  it is not a push or not minimally encoded (the script engine would refuse it). */
function readInt(h) {
  const op = parseInt(h.slice(0, 2), 16);
  if (op === 0) return [0n, 2];
  if (op === 0x4f) return [-1n, 2];
  if (op >= 0x51 && op <= 0x60) return [BigInt(op - 0x50), 2];
  if (!(op >= 1 && op <= 8) || h.length < 2 + op * 2) return null;
  const b = Buffer.from(h.slice(2, 2 + op * 2), 'hex');
  // minimal: no extra 0x00 / 0x80 top byte unless the next byte needs its sign bit
  if ((b[b.length - 1] & 0x7f) === 0 && (b.length === 1 || (b[b.length - 2] & 0x80) === 0)) return null;
  let v = 0n;
  for (let i = b.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(i === b.length - 1 ? b[i] & 0x7f : b[i]);
  v = b[b.length - 1] & 0x80 ? -v : v;
  // a value 1..16 or -1 has its own opcode: a data push of it is not minimal
  if ((v >= 1n && v <= 16n) || v === -1n) return null;
  return [v, 2 + op * 2];
}
function readForgeLock(s) {
  if (!s.startsWith(PRE) || !s.endsWith(POST)) return null;
  const n = readInt(s.slice(PRE.length));
  if (!n) return null;
  const rest = s.slice(PRE.length + n[1]);
  if (!rest.startsWith(MID) || rest.length !== MID.length + 64 + POST.length) return null;
  return { unlockDaa: n[0], beneficiary: rest.slice(MID.length, MID.length + 64) };
}

// Kaspa addresses (cashaddr-style bech32): prefix ':' base32(version ‖ payload ‖ checksum)
const CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
function polymod(values) {
  let c = 1n;
  for (const d of values) {
    const c0 = c >> 35n;
    c = ((c & 0x07ffffffffn) << 5n) ^ BigInt(d);
    if (c0 & 0x01n) c ^= 0x98f2bc8e61n;
    if (c0 & 0x02n) c ^= 0x79b76d99e2n;
    if (c0 & 0x04n) c ^= 0xf33e5fb3c4n;
    if (c0 & 0x08n) c ^= 0xae2eabe2a8n;
    if (c0 & 0x10n) c ^= 0x1e4f43e470n;
  }
  return c ^ 1n;
}
function to5(bytes) {
  const out = [];
  let acc = 0, bits = 0;
  for (const b of bytes) {
    acc = (acc << 8) | b; bits += 8;
    while (bits >= 5) { bits -= 5; out.push((acc >> bits) & 31); }
  }
  if (bits) out.push((acc << (5 - bits)) & 31);
  return out;
}
/** version 0: Schnorr public key (x-only, 32 bytes); version 8: pay-to-script-hash */
function address(prefix, version, payloadHex) {
  const data = to5([version, ...Buffer.from(payloadHex, 'hex')]);
  const pre = [...prefix].map((c) => c.charCodeAt(0) & 31);
  const sum = polymod([...pre, 0, ...data, 0, 0, 0, 0, 0, 0, 0, 0]);
  const check = [];
  for (let i = 7; i >= 0; i--) check.push(Number((sum >> BigInt(5 * i)) & 31n));
  return prefix + ':' + [...data, ...check].map((d) => CHARSET[d]).join('');
}

const args = process.argv.slice(2);
const feed = args[0] === '--file'
  ? JSON.parse(fs.readFileSync(args[1], 'utf8'))
  : await (await fetch(FEED + (args[0] ? '?token=' + encodeURIComponent(args[0]) : ''))).json();
let bad = 0, partial = 0;
for (const l of feed.locks || []) {
  const problems = [];
  let what = '', full = false;
  const c = l.contract || {};
  const s = String(c.redeemScript || '').toLowerCase();
  const holder = String(l.holder?.address || '');
  const prefix = holder.includes(':') ? holder.split(':')[0] : 'kaspa';
  if (!/^[0-9a-f]+$/.test(s) || s.length % 2) problems.push('no contract.redeemScript');
  else {
    const h = hash(s);
    if (h !== l.holder?.hash) problems.push('BLAKE2b-256(redeemScript) is not holder.hash');
    if (address(prefix, 8, h) !== holder) problems.push('holder.address is not the address of the script hash');
  }
  if (!problems.length && c.name === 'ForgeLock v3') {
    const f = readForgeLock(s);
    const arg = (n) => (c.args || []).find((a) => a.name === n)?.value;
    if (!f) problems.push('not the ForgeLock v3 layout');
    else {
      const ben = address(prefix, 0, f.beneficiary);
      if (f.beneficiary !== arg('beneficiary') || ben !== l.beneficiary) problems.push('the script releases to another key than the feed says');
      if (String(f.unlockDaa) !== String(arg('unlockDaa')) || String(f.unlockDaa) !== String(l.unlockDaa)) problems.push('the script unlocks at another DAA score than the feed says');
      what = `${l.amount} tokens unlock to ${ben} at DAA ${f.unlockDaa}`;
      full = true;
    }
  } else if (!problems.length) what = `${c.name || 'unknown contract'}: only the hash is checked here; rebuild it from source with ../rust`;
  if (problems.length) { bad++; console.log(`FAIL        ${l.id}  ${problems.join('; ')}`); }
  else if (!full) { partial++; console.log(`UNVERIFIED  ${l.id}  holder ${holder}\n            ${what}`); }
  else console.log(`OK          ${l.id}  holder ${holder}\n            ${what}`);
}
console.log(`\n${(feed.locks || []).length} locks checked, ${bad} failed, ${partial} need the full check (../rust)`);
process.exit(bad ? 1 : partial ? 2 : 0);
