# Weft — prepared deflation updates

Deflationary mechanics analogous to NeuralNS, built in and staged so they can be **announced
as updates** after launch. VPN is paid **only in $WEFT** (one price, no dual-rate problem).
Nothing here changes the launch — the buyback worker ships disabled and is turned on by a flag.

## What exists at launch (no action needed)

**On-chain burn on every $WEFT payment — already live.** Every VPN-traffic settlement splits
the payment **70% to the nodes** that carried the traffic, **20% burned on-chain** (real
`BurnChecked`, in the same transaction), **10% reserve**. Verifiable in every settlement tx.
This is the core "pay in token → supply leaves, on-chain, no trust" mechanic — on from day one.

**Live burn counter:** total burned = the mint's *initial supply − current supply* (read the
mint account directly). Every burn — payment burns and buyback burns — shows up there.

## Prepared update — SOL buyback & burn worker

`services/buyback-burn`. The team funds a buyback wallet with SOL (from the pump.fun buyback
proceeds); the worker spends it every ~10 min → buys $WEFT on Jupiter → sends the node share
to the payout pool and **burns the rest** (`BurnChecked`). No humans, no trust. It only touches
its own wallet, so it cannot overspend or affect the core program. This has **no exchange-rate
coupling to VPN pricing** — it is simply "burned N SOL worth of $WEFT."

Split of each buyback is configurable via `WEFT_BUYBACK_NODE_BPS` — `7000` = 70% tops up node
rewards / 30% burned; `0` = 100% burned (pure deflation).

Rollout:
1. `pnpm --filter @weft/buyback-burn bundle` → copy `dist/buyback-burn.mjs` to `/opt/weft/`.
2. `solana-keygen new -o /etc/weft/buyback.json`; **fund it with SOL** from the buyback proceeds.
3. `cp scripts/buyback.env.example /etc/weft/buyback.env`, fill in RPC + CA, set
   `WEFT_BUYBACK_ENABLE=1`.
4. `cp scripts/weft-buyback-burn.service /etc/systemd/system/`, `systemctl enable --now
   weft-buyback-burn`.
5. **Smoke test on mainnet with a small SOL amount** (Jupiter has no devnet liquidity): fund
   ~0.05 SOL, watch one cycle in `journalctl -u weft-buyback-burn` land a buy + split + burn.
6. Post the announcement.

## Announcement copy (ready to post)

**Burn, in the narrative**
> protocol update: every $WEFT payment burns on-chain.
> pay for VPN in $WEFT → 20% of the fee is burned in the same transaction. BurnChecked. not
> sent to a treasury, not burned later by hand. supply just leaves — verifiable in every tx.
> the rest pays the nodes that actually carry your traffic.

**Auto buyback & burn**
> $WEFT is now a deflation machine.
> pay in $WEFT → 20% burns on-chain, instantly, same transaction. BurnChecked.
> and the protocol auto-buys $WEFT on Jupiter and burns it — no humans, no trust,
> 100% automatic, verifiable on-chain.
> total burned so far: <supply delta>.
