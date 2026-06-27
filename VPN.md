# How to connect to Weft

## With a VLESS client you already use

Weft works with the popular VLESS clients — on your **phone or your computer**:
**V2Box**, **Happ**, **sing-box**, **Hiddify**, **Streisand**.

1. Install any one of them (free, from your app store or their site).
2. Get your **personal connection link** from the **cabinet** →
   [weftnetwork.net/app](https://www.weftnetwork.net/app): connect your wallet, open **access**.
   Weft is token-gated — your `$WEFT` balance is your traffic budget (1000 `$WEFT`/GB), so each
   wallet gets its own metered link. You get two: **1-hop** (fast, direct exit) and **multihop**
   (routed through the Tor network, onion, max privacy, slower). Copy one. When you've used what
   your `$WEFT` pays for, the link stops; top up (or earn `$WEFT` by running a node) and it
   resumes. On devnet, click **Get test $WEFT** in the cabinet to try it.

   _Scripted alternative:_ `curl -X POST https://<node>:8089/provision -d '{"wallet":"<PUBKEY>"}'`.

3. In the client: **＋ Add → Import from clipboard**, then **Connect**.

The link masquerades as ordinary HTTPS (VLESS + Reality), so it works even where VPNs are blocked.

## Did it work?

Open any "what is my IP" page. If it shows a different location than usual, you're connected and
your traffic is going through Weft.

---

There is no required Weft desktop app. VPN access uses standard VLESS clients; node operation uses
the repository scripts.

_Want to run a node and earn `$WEFT`? See the [README](README.md#run-a-node--earn)._
