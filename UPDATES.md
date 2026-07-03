# Weft — prepared deflation updates

Deflationary mechanics analogous to NeuralNS, built into the product and staged so they can
be **announced as updates** after launch. Nothing here changes the launch itself — the SOL
paths ship disabled and are enabled by flags when you decide to roll them out.

## What exists at launch (no action needed)

**On-chain burn on every $WEFT payment — already live.** Every VPN-traffic settlement splits
the payment **70% to the nodes** that carried the traffic, **20% burned on-chain** (real
`BurnChecked`, in the same transaction), **10% reserve**. Verifiable in every settlement tx.
This is the core "pay in token → supply leaves, on-chain, no trust" mechanic — on from day one.

**Live burn counter:** total burned = the mint's *initial supply − current supply* (read the
mint account directly). Every burn — payment burns and buyback burns — shows up there.

## Prepared updates (flag-gated OFF; enable when ready)

### Update A — SOL buyback & burn worker
`services/buyback-burn`. Spends a buyback wallet's SOL every ~10 min → buys $WEFT on Jupiter →
sends the node share to the payout pool and **burns the rest** (`BurnChecked`). No humans, no
trust. It only touches its own wallet, so it cannot overspend or affect the core program.

Rollout:
1. `pnpm --filter @weft/buyback-burn bundle` → copy `dist/buyback-burn.mjs` to `/opt/weft/`.
2. Create a buyback wallet: `solana-keygen new -o /etc/weft/buyback.json`; **fund it with SOL**
   from the pump.fun buyback proceeds.
3. `cp scripts/buyback.env.example /etc/weft/buyback.env`, fill in RPC + CA, set
   `WEFT_BUYBACK_ENABLE=1`.
4. `cp scripts/weft-buyback-burn.service /etc/systemd/system/`, `systemctl enable --now
   weft-buyback-burn`.
5. **Smoke test on mainnet with a small SOL amount** (Jupiter has no devnet liquidity): fund
   ~0.05 SOL, watch one cycle in `journalctl -u weft-buyback-burn` land a buy + split + burn.
6. Post Update A.

### Update B — pay for VPN in SOL
Backend (`SolPay`) + cabinet button, both flag-gated. Users send SOL to a collection wallet;
the control plane credits their quota; the collected SOL feeds Update A's buyback & burn. When
off, quota is escrow-only (unchanged). **Point the collection wallet at the buyback wallet** so
user SOL flows straight into buyback & burn.

Rollout:
1. Relay VPS `/etc/weft/node.env`: `WEFT_SOL_PAYMENTS=1`,
   `WEFT_SOL_COLLECTION=<buyback wallet>`, `WEFT_SOL_PRICE_PER_GB_LAMPORTS=10000000` (0.01
   SOL/GB), then `systemctl restart weft-control-plane`.
2. Frontend: redeploy with `VITE_WEFT_SOL_PAYMENTS=1`,
   `VITE_WEFT_SOL_COLLECTION=<buyback wallet>`, `VITE_WEFT_SOL_PRICE_PER_GB=0.01`.
3. **Test on devnet first**: send SOL to the collection wallet, confirm quota rises within a
   poll cycle and the link turns on.
4. Post Update B.

## Announcement copy (ready to post)

**Update A — burn goes live in the narrative**
> protocol update: every $WEFT payment burns on-chain.
> pay for VPN in $WEFT → 20% of the fee is burned in the same transaction. BurnChecked. not
> sent to a treasury, not burned later by hand. supply just leaves — verifiable in every tx.
> the rest pays the nodes that actually carry your traffic.

**Update B — the deflation machine**
> $WEFT is now a deflation machine. two ways it burns:
> pay in $WEFT → 20% burns on-chain, instantly, same transaction. BurnChecked.
> pay in SOL → the protocol auto-buys $WEFT on Jupiter every 10 minutes and burns it. no
> humans. no trust. 100% automatic, verifiable on-chain.
> total burned so far: <supply delta>.
