// Shared test fixtures: ed25519 signer generation and dual-signed receipt
// construction (so the address equals the public key the verifier checks).

import { ed25519 } from '@noble/curves/ed25519';
import { getAddressDecoder, type Address } from '@solana/kit';

import { signReceiptCore, type TrafficReceipt, type TrafficReceiptCore } from '../src/receipts';

const addrDec = getAddressDecoder();

export interface Signer {
  address: Address;
  secretKey: Uint8Array;
}

export function makeSigner(): Signer {
  const secretKey = ed25519.utils.randomPrivateKey();
  const pub = ed25519.getPublicKey(secretKey);
  return { address: addrDec.decode(pub), secretKey };
}

export function makeReceipt(
  client: Signer,
  operator: Signer,
  core: Omit<TrafficReceiptCore, 'client' | 'operator'>,
): TrafficReceipt {
  const full: TrafficReceiptCore = {
    client: client.address,
    operator: operator.address,
    ...core,
  };
  return {
    ...full,
    clientSig: signReceiptCore(full, client.secretKey),
    relaySig: signReceiptCore(full, operator.secretKey),
  };
}
