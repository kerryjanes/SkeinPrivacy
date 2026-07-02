# Weft — mainnet launch runbook

Coordinated launch: the pump.fun mint (CA) arrives ~10 min before go. Everything below
except the two launch commands is already prefilled. At go, run 2 commands, then fund + verify.

## State (prefilled — done)

| Piece | Value | Status |
|---|---|---|
| Program | `6riawCPVNE6sjMC6dgqkB2FxjXXFMXzuuy1pQRimk8Yd` | Deployed on mainnet, authority = admin, **uninitialized**. Same `.so` as source (sha `44f42b3a…`). Do NOT redeploy. |
| Admin / deploy authority | `9AY1on6okCYep7uKVTmJQbGHa7uqv5sQ4AwfuJhEkqJw` | Key at `~/.config/solana/weft-admin.json`. ~2.1 SOL on mainnet (launch needs < 0.1). Never copied to the VPS. |
| Program rent deposit | 4.40671704 SOL locked in ProgramData | **Recoverable** anytime via `solana program close` (see Recovery). |
| Frontend | https://weftnetwork.net (+ /app) | Mainnet build live; reads mint/vault/treasury/decimals from the on-chain distributor at runtime → **no rebuild at launch**. Faucet + keypair-login hidden. |
| Relay VPS | `root@13.140.2.111` | control-plane, aggregator, caddy, frps running (still devnet until cutover). |
| Poster + node-payout wallet | `DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq` | `/etc/weft/authority.json` on the VPS. Posts epochs + pays nodes directly. |
| Treasury | admin's ATA for the CA | Created automatically by `core:init`. Buyback $WEFT lands here. |
| RPC | Helius mainnet | Key read at runtime from `~/Documents/helius/config.json` (never committed). |

## Launch (on `go`, with the CA)

```bash
cd ~/Documents/Crypto/Weft
CA=<pump.fun mint>

# 1. On-chain: treasury ATA -> initialize_core(reward_mint=CA) -> provision cNFT collection+tree
./scripts/mainnet-launch.sh "$CA" --yes

# 2. Relay VPS: control-plane + aggregator -> mainnet + CA, wipe devnet state, restart
./scripts/mainnet-cutover.sh "$CA" --yes
```

Both are idempotent — safe to re-run if a step half-fails. Launch spend is only cNFT
tree rent + fees (< 0.1 SOL); the 4.4 SOL program deposit stays recoverable.

## Post-launch funding (before nodes withdraw)

Node rewards are paid directly from the payout wallet. After buyback $WEFT reaches the
treasury (admin ATA), forward some to the payout wallet:

```bash
spl-token transfer "$CA" <amount> DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq \
  --fund-recipient --url "$WEFT_RPC_URL" --owner ~/.config/solana/weft-admin.json
```

Solvency is safe by construction: payouts are capped at the wallet's balance, so an
underfunded wallet only *pauses* withdrawals — it can never overpay or go negative.

## Verify (after both commands)

- `curl https://vpn.weftnetwork.net:8089/price` → `mint` = CA, `faucet:false`.
- https://weftnetwork.net/app → connect a mainnet wallet; distributor resolves the token;
  deposit → copy `🇪🇺 Weft · Fast` link → connect → traffic meters and debits balance.
- Register a node from the cabinet → node cNFT lands in the wallet.
- Check treasury/reward-vault balances and a node's ATA before/after a withdrawal.

## Recovery (only if aborting mainnet entirely)

Returns the 4.4 SOL program deposit to the admin wallet (kills the deployed program):

```bash
solana program close 6riawCPVNE6sjMC6dgqkB2FxjXXFMXzuuy1pQRimk8Yd \
  --recipient 9AY1on6okCYep7uKVTmJQbGHa7uqv5sQ4AwfuJhEkqJw \
  -k ~/.config/solana/weft-admin.json --url mainnet-beta --bypass-warning
```
