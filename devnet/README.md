# Testing against uplink's devnet

Helpers for trying the module on [logos-blockchain/uplink](https://github.com/logos-blockchain/uplink)'s
local devnet (`just devnet up` there: four nodes, the LEZ sequencer and indexer, and the oracle).
They complement uplink's own `just devnet …` recipes, which only know devnet nodes; these bring in
what the module adds: Uplink invitation codes and the module's saved state.

| Script | Does |
|---|---|
| `register-under.sh <node> <code or file>` | Registers devnet node `<node>` under an inviter: an `uplink-invite-v1:` code (from `uplink_module.invitation`, e.g. the Uplink app's "Invite") or an invitation file the uplink CLI reads. Run it in the uplink checkout. Gives an Uplink user referrals to earn from. |
| `invite-to-cli.py <code>` | Turns an `uplink-invite-v1:` code into the uplink CLI's invitation JSON (`register-under.sh` does this itself). |
| `operations.py <uplink_state.json>` | Lists what the module sent for each deployment, from its saved state: registrations, claims (epochs, points) and cash-outs, with their status. |

```bash
cd <uplink checkout>
just devnet up && just devnet blend-join 2
sh <uplink-module>/devnet/register-under.sh 2 'uplink-invite-v1:…'
```

Each needs `python3`; `register-under.sh` also `just` and cargo, as uplink's recipes do.
