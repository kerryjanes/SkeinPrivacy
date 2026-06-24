#!/usr/bin/env bash
#
# Become a Weft node: run a Tor relay. Weft routes its traffic over the Tor network, so a
# Weft node IS a Tor relay — run one and you carry traffic for the network.
#
# By default this configures a NON-EXIT relay (a guard/middle relay): it relays traffic
# between other relays but never connects to the open internet on your behalf, so it is safe
# to run from a home connection. Pass --exit to run an exit relay (more demand and reward, but
# your IP egresses other people's traffic — understand the implications first).
#
# Usage (as root):
#   ./scripts/run-relay.sh                 # non-exit relay
#   ./scripts/run-relay.sh --exit          # exit relay (read the warning)
#   NICK=myrelay CONTACT=me@example.com ./scripts/run-relay.sh
#
set -euo pipefail

EXIT_RELAY=0
[ "${1:-}" = "--exit" ] && EXIT_RELAY=1

if [ "$(id -u)" -ne 0 ]; then echo "run as root (Tor binds a privileged ORPort)"; exit 1; fi

NICK="${NICK:-weft$(head -c4 /dev/urandom | od -An -tx1 | tr -d ' \n')}"
CONTACT="${CONTACT:-anonymous}"
ORPORT="${ORPORT:-9001}"

echo "→ Installing Tor…"
apt-get update -y >/dev/null
apt-get install -y tor >/dev/null

echo "→ Writing /etc/tor/torrc (relay: ${NICK}, ORPort ${ORPORT}, exit=${EXIT_RELAY})…"
cat > /etc/tor/torrc <<TORRC
Nickname ${NICK}
ORPort ${ORPORT}
ContactInfo ${CONTACT}
# Bandwidth you are willing to donate (raise these on a fast link).
RelayBandwidthRate 1 MBytes
RelayBandwidthBurst 2 MBytes
TORRC

if [ "$EXIT_RELAY" -eq 1 ]; then
  echo "⚠️  EXIT RELAY: your IP will appear as the source of other users' traffic."
  echo "    Only do this if you understand the abuse/legal implications for your connection."
  cat >> /etc/tor/torrc <<'TORRC'
ExitRelay 1
IPv6Exit 1
TORRC
else
  cat >> /etc/tor/torrc <<'TORRC'
ExitRelay 0
TORRC
fi

echo "→ Starting Tor…"
systemctl enable --now tor >/dev/null 2>&1 || service tor restart
sleep 3

# The relay fingerprint identifies your node on the network (and on-chain).
FP_FILE=/var/lib/tor/fingerprint
echo
if [ -f "$FP_FILE" ]; then
  echo "✅ Relay '${NICK}' is up. Fingerprint:"
  cat "$FP_FILE"
else
  echo "✅ Tor started. The fingerprint appears at ${FP_FILE} within a minute:"
  echo "   watch -n5 cat ${FP_FILE}"
fi
echo
echo "It takes the Tor network a few hours to measure a new relay before it carries much"
echo "traffic. Check status:  systemctl status tor   ·   journalctl -u tor -f"
echo
echo "Register the relay on-chain to earn \$WEFT (needs a funded Solana operator key):"
echo "  cd services/registry-provision && pnpm install && pnpm node:register"
echo "  (operator-side reward measurement from Tor relay metrics is on the roadmap)"
