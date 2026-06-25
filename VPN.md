# How to connect to Weft

## 1. With an app you already use (recommended)

Weft works with the popular VLESS clients — on your **phone or your computer**:
**V2Box**, **Happ**, **sing-box**, **Hiddify**, **Streisand**.

1. Install any one of them (free, from your app store or their site).
2. Get your **personal connection link**. Weft is token-gated: your `$WEFT` balance is your
   traffic budget (0.1 `$WEFT`/GB), so each wallet gets its own metered link from a node:

   ```sh
   curl -X POST http://<node>:8088/provision -d '{"wallet":"<YOUR_SOLANA_PUBKEY>"}'
   ```

   You get two links back — **1-hop** (fast, direct exit) and **multihop** (routed through the Tor
   network, onion, maximum privacy, slower). When you've used what your `$WEFT` pays for, the link
   stops; top up your wallet (or earn `$WEFT` by running a node) and it works again.

3. In the client: **＋ Add → Import from clipboard**, then **Connect**.

The link masquerades as ordinary HTTPS (VLESS + Reality), so it works even where VPNs are blocked.

## 2. The Weft app

Prefer one click? [Download the Weft app](https://github.com/kerryjanes/WeftNetwork/releases),
load your wallet, and press **Connect** — it provisions your personal link and shows your `$WEFT`
budget (balance, used, remaining) live. Nothing to paste.

## Did it work?

Open any "what is my IP" page. If it shows a different location than usual, you're connected and
your traffic is going through Weft.

---

_Want to run a node and earn `$WEFT`? See the [README](README.md#run-a-node) — it's one script._
