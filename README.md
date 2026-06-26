# Weft

Weft is a decentralized VPN on Solana.

Users pay `$WEFT` for traffic. Node operators run exit nodes, carry user traffic, and earn `$WEFT`
for the bytes they serve.

Weft does not require a custom VPN app. It uses standard **VLESS + Reality**, so users connect with
existing clients such as **V2Box**, **Happ**, **sing-box**, **Hiddify**, or **Streisand**.

## What You Can Do

- **Use VPN access:** connect a wallet, copy a personal VLESS link, import it into a VPN client.
- **Run a node:** register a node in the cabinet, run the shell script on your device, earn `$WEFT`.
- **Use 1-hop or multihop:**
  - **1-hop:** traffic exits through a Weft node. This is the fast mode.
  - **multihop:** traffic is routed through Tor by infrastructure nodes. This is slower, but more private.

## Use Weft As A VPN

1. Open [weftnetwork.net/app](https://www.weftnetwork.net/app).
2. Connect your Solana wallet.
3. Open the **access** tab.
4. Add `$WEFT` balance if needed.
5. Copy one of the main VPN links:
   - **1-hop:** fast direct exit through a Weft node,
   - **multihop:** slower Tor-routed mode for stronger privacy.
6. Import the `vless://` link into V2Box, Happ, sing-box, Hiddify, or Streisand.
7. Press **Connect** in that client.

Your VPN access is metered by `$WEFT`. When the prepaid budget runs out, the link stops. Add more
`$WEFT`, and it resumes.

If you are also running a node, the cabinet may show an extra **your node / your IP** link. That link
is mainly for operator testing, so you can verify that your own device works as an exit node.

## Run A Node

Your device can become a 1-hop exit node. Users connect through the public relay, but their traffic
exits from your device.

### 1. Register The Node

1. Open [weftnetwork.net/app](https://www.weftnetwork.net/app).
2. Connect the wallet that should own the node.
3. Open **deploy**.
4. Press **Register Node**.
5. Copy the generated **node key**.

Registration is the only on-chain step required to create the node.

### 2. Start The Node Script

On the device that should become the node:

```sh
curl -fsSL https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main/scripts/run-node.sh -o weft-node.sh
chmod +x weft-node.sh
./weft-node.sh <your-node-key>
```

After the first run, the key is saved locally:

```sh
./weft-node.sh
```

Stop the node:

```sh
./weft-node.sh stop
```

Remove local services and config:

```sh
./weft-node.sh stop --purge
```

The script installs and runs:

- Xray-core for VLESS + Reality,
- `frpc` for the reverse tunnel through the public relay,
- Weft control plane for metering and user access.

## Earn Rewards

When your node carries traffic, it becomes eligible for `$WEFT` rewards.

1. Keep the node online.
2. Wait for a settlement epoch to be posted.
3. Open the **rewards** tab in the cabinet.
4. Select your node and epoch.
5. Claim the reward to your wallet.

## Balances

- **SOL balance:** pays Solana transaction fees.
- **WEFT balance:** token balance in your wallet.
- **Prepaid escrow:** `$WEFT` reserved for VPN traffic. Unused prepaid balance can be withdrawn.

## Development

Requires Rust, Node 22+, pnpm, Anchor, and the Solana toolchain.

```sh
pnpm install
pnpm build
cargo test --workspace
anchor build
```

## Main Scripts

| Path | Purpose |
| --- | --- |
| `scripts/run-node.sh` | Run a home 1-hop node. |
| `scripts/stop-node.sh` | Legacy stop helper; `./weft-node.sh stop` is preferred. |
| `scripts/deploy-node.sh` | Deploy a VPS infrastructure node. |
| `scripts/test-home-exit.sh` | Local/devnet home-exit test helper. |

## License

[MIT](LICENSE)
