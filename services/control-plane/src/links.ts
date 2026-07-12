// Build a user's two personal connection links — identical shape to deploy-node.sh, but with a
// per-user UUID instead of one shared id.

import type { NodeConfig } from './config.js';

// Client-facing server names (the vless `#fragment`). The 🇪🇺 flag makes Happ/V2Box show a
// country badge instead of a blank one, and the label reads cleanly. URL-encoded so the
// emoji + spaces survive import.
const NAME_1HOP = encodeURIComponent('🇪🇺 Weft · Nodes');
const NAME_MULTIHOP = encodeURIComponent('🇪🇺 Weft · Tor');

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
  return `vless://${uuid}@${cfg.host}:${cfg.publicHop1Port}?${q}#${NAME_1HOP}`;
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
  return `vless://${uuid}@${cfg.host}:${cfg.publicHopnPort}?${q}#${NAME_MULTIHOP}`;
}
