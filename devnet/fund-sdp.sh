#!/bin/sh
# Tops up the SDP funding keys of uplink devnet nodes (logos-blockchain/uplink) so their Blend
# activity messages keep getting paid. Genesis gives each 10000, and each message costs ~2400-3600,
# so without this the nodes fall out of the Blend membership a few epochs after `blend-join`, and
# nobody earns referral points. A workaround until uplink's devnet genesis funds them for longer.
#
# Run it in the uplink checkout right after `just devnet up`:
#   sh <uplink-module>/devnet/fund-sdp.sh [nodes...]      (default: 0 1 2)
# DEVNET_HOST reaches a devnet on another machine.
#
# The money comes from node 3's stake note: not node 0's (it holds 99% of the stake and makes the
# blocks; spending it stalls the chain) and not the sequencer's funding key (spending its notes
# stops it writing LEZ blocks to the chain, and the indexer with it). Keys are derived from the
# node index, so they're the same on every devnet.
set -eu

SOURCE=a8637f8c9b98a198117db999552ceb57d64dff4eaf950c6d4ff1034aaf050d24   # node 3's stake key
AMOUNT=100000000
state="$PWD/devnet/state/config"
[ -d "$state/node-3" ] || { echo "Run this in the uplink checkout, with the devnet up." >&2; exit 1; }
nodes=${*:-0 1 2}

api() { echo "http://${DEVNET_HOST:-127.0.0.1}:1808$1"; }
sdp_key() { grep "funding_pk" "$state/node-$1/config.yaml" | tail -n1 | awk '{print $NF}'; }
balance() { curl -s "$(api "$1")/wallet/$2/balance" | sed -n 's/.*"balance":\([0-9]*\).*/\1/p'; }

until curl -sf "$(api 3)/cryptarchia/info" >/dev/null; do echo "waiting for node 3…"; sleep 2; done

for n in $nodes; do
    key=$(sdp_key "$n")
    before=$(balance "$n" "$key"); before=${before:-0}
    echo "node $n: SDP key ${key%"${key#????????}"}… has $before"
    until curl -s -X POST "$(api 3)/wallet/transactions/transfer-funds" -H 'content-type: application/json' \
            -d "{\"tip\":null,\"change_public_key\":\"$SOURCE\",\"funding_public_keys\":[\"$SOURCE\"],\"recipient_public_key\":\"$key\",\"amount\":$AMOUNT}" \
            | grep -q '"hash"'; do
        echo "  node 3's change from the last transfer isn't confirmed yet; retrying in 5 s"; sleep 5
    done
    until [ "$(balance "$n" "$key")" -ge $((before + AMOUNT)) ] 2>/dev/null; do sleep 3; done
    echo "  now $(balance "$n" "$key")"
done
