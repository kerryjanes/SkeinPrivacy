#!/bin/sh
# The container's xray supervisor (node-entrypoint.sh) watches xray.json's mtime and restarts
# xray whenever the control-plane rewrites it, so this reload hook is intentionally a no-op.
exit 0
