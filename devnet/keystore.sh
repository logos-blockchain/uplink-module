#!/bin/sh
# Writes the keystore.yaml the Blockchain app signs from (logos-blockchain-ui reads it beside the
# node's config.yaml) for uplink devnet nodes. The devnet keeps each node's Blend signing key as
# node.key and its public key as provider_id; this puts them in the keystore format, readable by
# its owner only. `just devnet reset` deletes it with the rest of devnet/state, so run it again
# after each one.
#
# Run it in the uplink checkout, with the devnet up (`just devnet up`):
#   sh <uplink-module>/devnet/keystore.sh [nodes...]      (default: 1)
# Then point the Blockchain app at devnet/state/config/node-<n>/config.yaml.
set -eu

state="$PWD/devnet/state/config"
[ -d "$state/node-0" ] || { echo "Run this in the uplink checkout, with the devnet up." >&2; exit 1; }
nodes=${*:-1}

for n in $nodes; do
    d="$state/node-$n"
    if [ ! -s "$d/node.key" ] || [ ! -s "$d/provider_id" ]; then
        echo "node $n: no node.key and provider_id in $d" >&2
        exit 1
    fi
    node=$(tr -d ' \n' < "$d/provider_id")
    # In a subshell, so only the keystore is created owner-only. The secret goes from file to
    # file and is never printed.
    (
        umask 077
        printf 'public_keys:\n  BlendSigning: %s\nsecret_keys:\n  BlendSigning: !Ed25519 %s\n' \
            "$node" "$(tr -d ' \n' < "$d/node.key")" > "$d/keystore.yaml"
    )
    echo "node $n: $d/keystore.yaml (node $(printf %s "$node" | cut -c1-8)…)"
done
