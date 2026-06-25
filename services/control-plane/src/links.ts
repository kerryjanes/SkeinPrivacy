// Build a user's two personal connection links — identical shape to deploy-node.sh, but with a
// per-user UUID instead of one shared id.

import type { NodeConfig } from './config.js';

export function oneHopLink(cfg: NodeConfig, uuid: string): string {
  const q = new URLSearchParams({
    flow: 'xtls-rprx-vision',
    type: 'tcp',
    security: 'reality',
    fp: 'firefox',
    sni: cfg.sni,
    pbk: cfg.realityPublicKey,
    sid: cfg.shortId,
    spx: '/',
  });
  return `vless://${uuid}@${cfg.host}:${cfg.hop1Port}?${q}#Weft-1hop`;
}

export function multiHopLink(cfg: NodeConfig, uuid: string): string {
  const q = new URLSearchParams({
    type: 'tcp',
    security: 'reality',
    fp: 'firefox',
    sni: cfg.sni,
    pbk: cfg.realityPublicKey,
    sid: cfg.shortId,
    spx: '/',
  });
  return `vless://${uuid}@${cfg.host}:${cfg.hopnPort}?${q}#Weft-multihop`;
}
