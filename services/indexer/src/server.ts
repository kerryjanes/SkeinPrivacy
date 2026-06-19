import { createServer, type Server } from 'node:http';

import { fetchNodes, filterNodes, rpc, type NodeQuery, type NodeRecord } from './directory';

const CACHE_TTL_MS = 10_000;

export function startServer(rpcUrl: string, port: number): Server {
  const client = rpc(rpcUrl);
  let cache: { at: number; nodes: NodeRecord[] } | null = null;

  async function nodes(): Promise<NodeRecord[]> {
    if (!cache || Date.now() - cache.at > CACHE_TTL_MS) {
      cache = { at: Date.now(), nodes: await fetchNodes(client) };
    }
    return cache.nodes;
  }

  const server = createServer((req, res) => {
    void (async () => {
      try {
        const url = new URL(req.url ?? '/', 'http://localhost');
        if (url.pathname === '/health') {
          res.writeHead(200).end('ok');
          return;
        }
        if (url.pathname === '/nodes') {
          const p = url.searchParams;
          const q: NodeQuery = {};
          if (p.get('availability')) q.minAvailability = Number(p.get('availability'));
          if (p.get('minReputation')) q.minReputation = Number(p.get('minReputation'));
          if (p.get('capability')) q.capability = Number(p.get('capability'));
          if (p.get('geo'))
            q.geoPrefix = { region: Number(p.get('geo')), chars: Number(p.get('geoChars') ?? 2) };
          const result = filterNodes(await nodes(), q);
          res.writeHead(200, { 'content-type': 'application/json' });
          res.end(JSON.stringify({ count: result.length, nodes: result }));
          return;
        }
        res.writeHead(404).end('not found');
      } catch (e) {
        res.writeHead(500, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ error: String(e) }));
      }
    })();
  });
  server.listen(port);
  return server;
}
