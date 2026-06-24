# Connecting to Weft

Weft speaks **VLESS**, the protocol that **V2Box, Happ, sing-box and Hiddify** already speak. You
connect with a standard client you already trust — no Weft-specific software required. Your
traffic is onion-routed through 3–5 nodes before it exits, and no hop can see both who you are and
where you're going.

## What you need

1. A **VLESS client** for your platform:
   - **V2Box** — iOS, Android
   - **Happ** — iOS, Android, macOS
   - **sing-box** — macOS, Windows, Linux
   - **Hiddify** — Windows, macOS, Linux, iOS, Android
2. A **`vless://` link** to a Weft node (from a node operator, a public directory, or your own node).

## Connect

### Phone — V2Box / Happ

1. Install the app from the App Store or Google Play.
2. Copy your `vless://…` link.
3. Open the app → **＋ Add** (or the import button) → **Import from clipboard**.
4. Tap the new profile → **Connect**.

### Desktop — sing-box / Hiddify

1. Install sing-box or Hiddify.
2. Add the `vless://…` link as a new profile (Hiddify: paste it → **Add**; sing-box: add it as a
   `vless` outbound in your config).
3. Start the tunnel.

## Getting a `vless://` link

- Ask a node operator, or pull one from the public node directory.
- Or run your own node and let it print a link:

  ```sh
  cargo run -p weft-vpn --release -- vless 0.0.0.0:8443 --tls
  ```

  This starts a VLESS gateway and prints a `vless://…` link. To connect a phone on the same Wi-Fi,
  replace the host in the link with your machine's LAN IP. For access from anywhere, run the node
  on a host with a public IP.

## Verify it works

After connecting, open <https://ipinfo.io> (or any "what is my IP" page). It should show the Weft
**exit's** address, not your own. That confirms your traffic is leaving through the network.

## Notes

- Links carry `security=none` or `security=tls`. For a self-signed TLS node while testing, enable
  **Allow insecure** in your client.
- Both TCP and UDP (so DNS) are supported.

## Prefer one click?

The **Weft desktop app** bundles the engine — [download it from Releases](https://github.com/kerryjanes/WeftNetwork/releases),
open it, and press **Connect**. No link to paste.
