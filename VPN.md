# How to connect to Weft

## 1. With an app you already use (recommended)

Weft works with the popular VLESS clients — on your **phone or your computer**:
**V2Box**, **Happ**, **sing-box**, **Hiddify**, **Streisand**.

1. Install any one of them (free, from your app store or their site).
2. Get a **connection link** (`vless://…`) for a node. Each node gives you two:
   - **1-hop** — fast, direct exit.
   - **multihop** — routed through the Tor network (onion, maximum privacy, slower).
   (Running your own node? `./scripts/deploy-node.sh` prints both links.)
3. In the client: **＋ Add → Import from clipboard**, then **Connect**.

The link masquerades as ordinary HTTPS (VLESS + Reality), so it works even where VPNs are blocked.

## 2. The Weft app

Prefer one click? [Download the Weft app](https://github.com/kerryjanes/WeftNetwork/releases),
open it, and press **Connect**. Nothing to paste.

## Did it work?

Open any "what is my IP" page. If it shows a different location than usual, you're connected and
your traffic is going through Weft.

---

*Want to run a node and earn `$WEFT`? See the [README](README.md#run-a-node) — it's one script.*
