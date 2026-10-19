# uplink_module

The Uplink node referral program as a Logos module: terms, enrolment, claims and cash-out
against the referral program in [logos-blockchain/uplink](https://github.com/logos-blockchain/uplink),
through `lez_core`'s generic API. The Uplink app in Basecamp
([logos-uplink-app](https://github.com/logos-co/logos-uplink-app)) and `logoscore` call the
same methods.

> Status: early. `lez_core` is pinned to an open PR (see [Pins](#pins)), and the program has
> so far only run on a local devnet.

## How it fits

```
Uplink app / logoscore ──▶ uplink_module ──▶ lez_core             (wallet: keys, reads, proving, submit)
                                 │       └─▶ lez_indexer_module   (optional: past publications)
                                 └─ referral_core (uplink)        (the program's formats, compiled in)
```

A Rust module (`codegen.rust`), one crate in `rust-lib/`:

| Path | Does |
|---|---|
| `src/domain/` | Pure: decode accounts, the enrolment message, the keystore, instructions, claim batching, cash-out secrets, invitations, publications, terms. |
| `src/ports.rs` | Traits for what the service needs from outside: `Lez`, `History`, `Store`, `Files`, `Clock`. |
| `src/service/` | The use cases and the saved state (`saved.rs`). Tested against fakes in `tests.rs`, and against the real program in `program_tests.rs`. |
| `src/adapters.rs` | The real ports: `modules().lez_core`, `modules().lez_indexer_module`, the data directory, files. |
| `src/glue.rs` | The module API (the `UplinkModule` trait). |
| `assets/terms.json` | The program's terms, verbatim from the T&Cs draft of 2026-08-07. |
| `build.rs` | Finds the program binary: uplink's `artifacts/programs/referral/referral.bin` at the `referral_core` pin. |

## Keys

The node's Blend signing key (`BlendSigning` in its `keystore.yaml`; its public half is the
node's ID in the program) signs the enrolment and derives the cash-out receipt secret, as
uplink defines it (`SHA256("Uplink/ReceiptSecret/v1\0" ‖ node secret)`), so receipts match
the `uplink` CLI's. The caller passes the path to the node's `keystore.yaml`, which is the
user's consent. The module reads only the Ed25519 key whose public key is the node, uses it,
and wipes it. It is never stored, logged or returned. Nothing here signs anything but an
enrolment, a payout code, or a receipt secret.

## API

Every method returns JSON: `{"ok": true, …}` or `{"ok": false, "code", "error"}`. Ids are
hex. `reference` is a caller-chosen 32-byte hex id; resubmitting one returns its record
instead of sending again.

| Method | Returns |
|---|---|
| `info()` | selected program account, terms version, deployments seen, `stateProblem` (below) |
| `configure(program_account)` | same as `info`; `""` clears it |
| `check_program()` | `{programAccount, imageId}` if that account runs this module's program; else `not_configured` / `program_not_deployed` / `program_mismatch` |
| `terms()` / `terms_status()` / `accept_terms(version, sha256)` | the text + hash / whether accepted / acceptance |
| `create_participant(label)` / `find_participant(label)` | the participant account |
| `participant(participant)` | `{registered, node, referrer, children, childrenSince, rewardBalance}`; `childrenSince` = each referral's join epoch, remembered from its join note |
| `invitation(participant, node)` / `import_invitation(participant, blob)` | an invitation code / the referrer |
| `check_invitation(blob)` | `{referrer, registered}`, without saving |
| `registry()` | `{epoch, nodes, active}` |
| `activity(nodes, count)` | per node (comma-separated hex), active in each of the last `count` epochs: `1` / `0` / `-1` unknown |
| `enrol(reference, participant, keystore_path, node)` | the Register operation; `node` = the running node, so a keystore for another node is refused (`""` = the keystore's BlendSigning key) |
| `enrolment_message(participant, node)` / `enrol_with_signature(reference, participant, node, signature)` | for signing elsewhere |
| `keystore_node(keystore_path)` | the node a keystore is for, from its public keys only |
| `status(reference)` / `operations(participant)` | `submitting` → `pending` → `settled`, or `failed`; a transaction no block includes within 100 blocks fails (a Register found on chain settles instead), so it can be retried |
| `sync()` / `notes(participant)` | syncs the wallet / new referrals and credits |
| `claimable(participant)` / `claim(reference, participant)` | points by batch / submits the oldest batch (≤16 epochs) |
| `auto_claim(participant)` | claims one batch when nothing is pending; call it on every poll |
| `cash_out(reference, participant, keystore_path)` | the CashOut operation (whole balance) |
| `receipts(participant, keystore_path)` | receipts; with `""` the ones already known, without reading the key |
| `payout_code(participant, index, keystore_path)` | the signed `uplink-payout-v2:` code |

Epochs can be claimed late: each published epoch is archived on-chain, and each child is
paid once per epoch whenever it's claimed. The archive holds only a digest of each epoch's
active set: the module learns the set itself live, or from `lez_indexer_module` (checked
against the digest). Without either, those claims fail with `history_needed`. Invitation
codes are `uplink-invite-v1:` + base64url of the inviter's node and its participant's public
keys; the `uplink` CLI's invitation JSON is accepted too.

### From `logoscore`

```bash
logoscore … -c "uplink_module.configure(<program account>)"                    # once
logoscore … -c "uplink_module.check_program()"
logoscore … -c "uplink_module.terms()"
logoscore … -c "uplink_module.accept_terms(1, <sha256>)"
logoscore … -c "uplink_module.create_participant(uplink-referral)"
logoscore … -c "uplink_module.import_invitation(<participant>, <invitation code>)"   # if invited
logoscore … -c "uplink_module.enrol(<reference>, <participant>, /path/to/keystore.yaml, <node>)"
logoscore … -c "uplink_module.status(<reference>)"
```

## Configuration

The only setting is which deployment to use: the account the referral program was deployed
at (its loader header account, hex or base58), from `configure(program_account)` or, when
nothing is configured, `UPLINK_PROGRAM` (the variable the `uplink` CLI reads). The module
picks no network itself: an app passes its release's deployment (the Uplink app does on every
refresh), a CLI user configures it once.

The program binary is embedded: uplink's committed `artifacts/programs/referral/referral.bin`,
taken from uplink at the commit `referral_core` is pinned to in `rust-lib/Cargo.lock`
(`build.rs`; the nix build fetches uplink at that commit, a plain `cargo` build reads cargo's
own checkout of it), so moving the pin moves the binary with it. Before sending, the module
checks the deployed program's image id (from its header) against the embedded binary's, so a
wrong network or another version fails with `program_not_deployed` / `program_mismatch`
instead of a rejected transaction.

Each program account is its own deployment (devnet, testnet, …) with its own saved state in
`uplink_state.json` (the module's data directory): invitations, operations, epoch lists,
join epochs and terms acceptance. Calling `configure` with another program account switches
to it; switching back finds the earlier state as it was. `info()` lists the deployments seen.
If the file doesn't parse, the module moves it aside (`uplink_state.json.unreadable-<ms>`),
starts empty, and `info()` reports `stateProblem: {code: "state_set_aside", file}`. If it
can't be read at all, nothing is saved over it: calls that save fail with `state_unreadable`
until it's fixed.

Sending a private transaction means proving it, which takes minutes without
`RISC0_DEV_MODE`, so the module waits up to 30 minutes for `lez_core` there; callers should
call `enrol`, `claim`, `auto_claim` and `cash_out` without the default 20 s timeout.

## Build and test

```bash
cd rust-lib && cargo test --no-default-features   # domain + service, no host needed
nix build                                          # the module
```

`cargo` needs `logos-rust-sdk-src` next to `rust-lib` (a symlink to `logos-rust-sdk`;
`nix build` stages its own).

## Pins

| What | Pin | Status |
|---|---|---|
| `referral_core`, `referral_program` (tests), the program binary | logos-blockchain/uplink `master` @ `8021f33` | merged |
| LEZ crates (`lee_core`, `indexer_service_protocol`) | tag `v0.3.1`, as uplink pins them | released |
| `lez_core` | logos-execution-zone-module PR #69 @ `dde621a` (0.5.2): `send_generic_private_transaction_with_accounts` | open |
| `lez_indexer_module` | `87463b1` (1.2.1) | released |

Update the pins when the PRs merge.
