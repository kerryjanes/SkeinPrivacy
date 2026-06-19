import { startServer } from './server';

const port = Number(process.env.PORT ?? 8787);
const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
startServer(rpcUrl, port);
console.log(`[indexer] node directory on :${port} (rpc ${rpcUrl})`);
