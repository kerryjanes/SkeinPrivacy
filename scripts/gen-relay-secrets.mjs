#!/usr/bin/env node
// Generate fresh relay secrets for a Weft node/relay — the Reality x25519 identity, founder
// credential, and the relay/receipts auth tokens. Output is an env file the cutover sources
// (default ~/.config/weft/relay-secrets.env). NEVER commit the result; these are secrets.
//
//   node scripts/gen-relay-secrets.mjs > ~/.config/weft/relay-secrets.env
//
// The x25519 keys are emitted as base64url raw (the exact format Xray Reality expects). Rotating
// them rotates the live node identity: after regenerating, re-run the cutover so the control-plane
// re-renders Xray with the new key. Old/leaked values are burned the moment they are public.
import crypto from 'node:crypto';

const { publicKey, privateKey } = crypto.generateKeyPairSync('x25519');
const priv = privateKey.export({ format: 'jwk' }).d; // base64url raw 32-byte scalar
const pub = publicKey.export({ format: 'jwk' }).x; // base64url raw 32-byte point

const lines = [
  '# Weft relay secrets — generated, uncommitted. Keep this file private (chmod 600).',
  `WEFT_REALITY_PBK=${pub}`,
  `WEFT_REALITY_PRIV=${priv}`,
  `WEFT_SID=${crypto.randomBytes(8).toString('hex')}`,
  `WEFT_FOUNDER_UUID=${crypto.randomUUID()}`,
  `WEFT_RELAY_TOKEN=${crypto.randomBytes(16).toString('hex')}`,
  `WEFT_RECEIPTS_TOKEN=${crypto.randomBytes(32).toString('hex')}`,
];
process.stdout.write(lines.join('\n') + '\n');
