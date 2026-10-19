#!/bin/sh
# Registers an uplink devnet node under an inviter: an Uplink user (an uplink-invite-v1: code, from
# uplink_module.invitation) or anyone with an invitation file the uplink CLI reads. It does what
# `just devnet register` does in logos-blockchain/uplink, but with an invitation from outside the
# devnet. Run it in the uplink checkout, with the devnet up (`just devnet up`):
#
#   sh <uplink-module>/devnet/register-under.sh 2 'uplink-invite-v1:…'
#   sh <uplink-module>/devnet/register-under.sh 2 invitation.json
set -eu

n=${1:?usage: $0 <node index> <invitation code or file>}
invitation=${2:?usage: $0 <node index> <invitation code or file>}
state="$PWD/devnet/state"
[ -s "$state/program" ] || { echo "Run this in the uplink checkout, with the devnet up." >&2; exit 1; }
case "$invitation" in
    uplink-invite-v1:*)   # a code: the CLI's file, next to the devnet's own invitations
        file="$state/invitations/node-$n-invited.json"
        mkdir -p "$state/invitations"
        python3 "$(dirname "$0")/invite-to-cli.py" "$invitation" > "$file"
        invitation=$file ;;
esac
[ -s "$invitation" ] || { echo "$invitation is missing or empty." >&2; exit 1; }

referrer=$(python3 -c 'import json, sys; print(bytes(json.load(open(sys.argv[1]))["parent_node"]).hex())' "$invitation")
node=$(tr -d ' \n' < "$state/config/node-$n/provider_id")

uplink() {   # the uplink CLI, acting as node n's operator
    RISC0_DEV_MODE=1 UPLINK_INDEXER=http://127.0.0.1:8779 UPLINK_PROGRAM="$(cat "$state/program")" \
        LEE_WALLET_HOME_DIR="$state/wallets/node-$n" cargo run -q -p uplink -- "$@"
}

echo "Registering node $n ($node) under node $referrer"
# Captured before `tail`: in a pipeline, a failure would be tail's success.
me=$(just devnet participant "$n")
me=$(printf '%s\n' "$me" | tail -n1)
[ -n "$me" ] || { echo "No participant for node $n." >&2; exit 1; }
echo "node $n's participant: $me"
uplink accept-invite --participant "$me" "$invitation"
msg=$(uplink prepare-registration --participant "$me" --node "$node" --referrer "$referrer")
msg=$(printf '%s\n' "$msg" | tail -n1)
sig=$(RISC0_DEV_MODE=1 cargo run -q -p uplink_devnet -- sign --key-file "$state/config/node-$n/node.key" "$msg")
uplink attach-signature --participant "$me" --signature "$sig"
uplink register --participant "$me"
just devnet show "$n"
