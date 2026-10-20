# uplink_module

The Uplink node referral program as a Logos module: terms, enrolment, claims and cash-out
against the referral program in [logos-blockchain/uplink](https://github.com/logos-blockchain/uplink),
through `lez_core`'s generic API. The Uplink app in Basecamp
([logos-uplink-app](https://github.com/logos-co/logos-uplink-app)) and the command line
(`logosctl`) call the same methods.

> Status: early. `lez_core` is pinned to an open PR (see [Pins](#pins)), and the program has
> so far only run on a local devnet.

## How it fits

```
Uplink app / logosctl ──▶ uplink_module ──▶ lez_core             (wallet: keys, reads, proving, submit)
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
node's ID in the program) provides three things: the enrolment's signature, the payout
code's signature, and the cash-out receipt secret, as uplink defines it
(`SHA256("Uplink/ReceiptSecret/v1\0" ‖ node secret)`, so receipts match the `uplink` CLI's).

Every one of them can come from whoever holds the key, so the key never reaches the module.
In Basecamp that's the Blockchain app, which asks the user each time:

| Needs | Module calls |
|---|---|
| Enrolment signature | `enrolment_message` → sign → `enrol_with_signature` |
| Receipt secret | `cash_out_with_secret`, `receipts_with_secret` |
| Payout signature | `payout_message` → sign → `payout_code_with_signature` |

The module checks what it's handed: a signature must be the node's over that exact message,
and a receipt secret must be the one first used for the participant (it keeps a fingerprint,
never the secret) and find the receipts already known. A wrong secret would otherwise put a
cash-out where no one could find it again.

On the command line, the same calls can take the path to the node's `keystore.yaml` instead
(`enrol`, `cash_out`, `receipts`, `payout_code`, and `keystore_node`), but only where the
module runs with `UPLINK_KEYSTORE_PATHS=1`: any caller can name a path, so by default the
module reads no keystore at all and answers `keystore_paths_off`. The module reads only the Ed25519 key whose
public key is the node, uses it, and wipes it. It is never stored, logged or returned.

## API

Every method returns JSON: `{"ok": true, …}` or `{"ok": false, "code", "error"}`. Ids are
hex. A `participant` may also be given in base58, upper case or with `0x`; replies always
use lowercase hex. `reference` is a caller-chosen 32-byte hex id; resubmitting one returns its record
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
| `status(reference)` / `operations(participant)` | `submitting` → `pending` → `settled`, or `failed`. Answers at once: a pending operation settles when the indexer has its transaction or the chain shows its outcome (registered, points added, receipt created); one still out after 100 blocks fails, so it can be retried |
| `sync()` / `notes(participant)` | syncs the wallet / new referrals and credits |
| `claimable(participant)` / `claim(reference, participant)` | points by batch, and `history` (below) / submits the oldest batch (≤16 epochs) |
| `auto_claim(participant)` | claims one batch when nothing is pending; call it on every poll. A failed batch is retried under the same reference after 10, 20, 40, … (up to 640) blocks: `{skipped: "backoff", retryAtBlock}` meanwhile |
| `cash_out(reference, participant, keystore_path)` | the CashOut operation (whole balance) |
| `receipts(participant, keystore_path)` | receipts; with `""` the ones already known, without reading the key |
| `payout_code(participant, index, keystore_path)` | the signed `uplink-payout-v2:` code |
| `cash_out_with_secret(reference, participant, receipt_secret)` / `receipts_with_secret(participant, receipt_secret)` | the same, with the node's receipt secret (hex) instead of its keystore |
| `payout_message(participant, index, receipt_secret)` / `payout_code_with_signature(participant, index, signature)` | the text the node signs for a payout code (`""` secret for a receipt already known) / the code from that signature |

### Claim history

Epochs can be claimed late, and in any order: each published epoch is archived on-chain,
and each child is paid once per epoch whenever it's claimed. But the archive holds only a
digest of each epoch's active set, and a claim must name the set itself (the program checks
it against the digest). The module learns each set live, while the epoch is current, or
reads it from `lez_indexer_module` (checked against the digest).

Live means the recorder: a thread that looks at the registry for as long as the module is
loaded, whether or not an app is calling it (in Basecamp, also with the Uplink app closed):
every 15 s after an epoch change, slowing to every 10 minutes as the epoch ages (the
registry is a few MB on a large network). It saves each epoch's set to
`uplink_epochs.json`, which the module takes in on its next registry read, and then forgets
it. So epochs pass unclaimed without loss while Basecamp runs; the
indexer is for the time it didn't.

Only the sets a claim still needs are kept whole: an epoch's set is dropped once no
participant's latest claim plan claims it (the two most recent epochs always stay, for
referrals' late join notes). What an epoch pays depends only on whether you and your
referrals were active, so the module notes that for every epoch it learns (your node, your
referrer, your referrals and any node `activity` was asked about) and plans and draws the
activity strips from those notes. If an old epoch becomes claimable again (a referral's late
claim forwards a credit for it), its set comes back from the indexer in one call: the module
remembers where each publication is in the indexer's history.

An epoch whose set it has neither doesn't hold up the others: they're claimed, and it's
reported in `claimable`'s `history` as `{code, missingEpochs, pointsWaiting, indexer:
{state, indexedBlock, error}}` (`pointsWaiting` is what the module can already tell those
epochs pay, so at least that much):

- `history_syncing`: the indexer is running but still catching up. The epochs will come.
- `history_needed`: no indexer is running (or it failed). Start `lez_indexer_module`; in
  Basecamp, from the LEZ Explorer app.

`claim` fails with the same code only when those epochs are all there is to claim, and
`auto_claim` answers `{skipped: <code>, history}`. A fresh indexer replays the chain from
genesis against your node, so it finds every past epoch, also after restoring a wallet on a
new machine (this relies on nodes keeping all finalized blocks, as they do while they sync
from genesis; a node bootstrapped from a checkpoint wouldn't have the epochs before it).

From the command line, the indexer (and the recorder) run as long as the daemon that
loaded them:

```bash
logosctl daemon start --detach
logosctl module load uplink_module          # with lez_core, and lez_indexer_module (optional deps)
logosctl call lez_indexer_module start_indexer /abs/path/indexer_config.json
logosctl call uplink_module claimable <participant>   # again until history is null
logosctl call uplink_module claim <reference> <participant>
```

Invitation codes are `uplink-invite-v1:` + base64url of the inviter's node and its participant's public
keys; the `uplink` CLI's invitation JSON is accepted too.

### From the command line

With the daemon running (`logoscore` takes the same calls), and allowed to read the
keystore, since there's no Blockchain app to sign:

```bash
UPLINK_KEYSTORE_PATHS=1 logosctl daemon start --detach
```

```bash
logosctl module load uplink_module
logosctl call uplink_module configure <program account>                 # once
logosctl call uplink_module check_program
logosctl call uplink_module terms
logosctl call uplink_module accept_terms 1 <sha256>
logosctl call uplink_module create_participant uplink-referral
logosctl call uplink_module import_invitation <participant> <invitation code>   # if invited
logosctl call uplink_module enrol <reference> <participant> /path/to/keystore.yaml <node>
logosctl call uplink_module status <reference>
```

Arguments that look like numbers are sent as numbers; pass a hex id that is all digits as
`str:<id>`.

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
`uplink_state.json` (the module's data directory): invitations, operations, join epochs and
terms acceptance. Epoch active sets, a few MB each on a large network, are kept apart in
`uplink_lists.json`, a cache the indexer can refill. Calling `configure` with another program account switches
to it; switching back finds the earlier state as it was. `info()` lists the deployments seen.
If the file doesn't parse, the module moves it aside (`uplink_state.json.unreadable-<ms>`),
starts empty, and `info()` reports `stateProblem: {code: "state_set_aside", file}`. If it
can't be read at all, nothing is saved over it: calls that save fail with `state_unreadable`
until it's fixed.

Sending a private transaction means proving it, which takes minutes without
`RISC0_DEV_MODE`, so the module waits up to 30 minutes for `lez_core` there; callers should
call `enrol`, `claim`, `auto_claim` and `cash_out` without the default 20 s timeout.

While a send is proving, the module answers nothing else, and that's by design rather than
a missing `"concurrency": "multi"`. `lez_core` runs one call at a time and its wallet stays
locked through the proof (`wallet_ffi_send_generic_private_transaction`), so any other call
that reads the wallet would wait behind it anyway. Callers should not poll during a send
(the Uplink app pauses its refresh); the result comes back from the send itself, or from
`status(reference)` afterwards.

## Build and test

```bash
cd rust-lib && cargo test --no-default-features   # domain + service, no host needed
nix build                                          # the module
```

`cargo` needs `logos-rust-sdk-src` next to `rust-lib`, a checkout (or symlink) of
`logos-rust-sdk` at the revision in `flake.lock`, even with the module feature off; `nix
build` stages its own. CI (`.github/workflows/ci.yml`) runs both on every PR: `nix build` on
Linux and macOS, and the cargo tests on Linux.

To try the module on uplink's local devnet, see [`devnet/`](devnet/README.md).

## Pins

| What | Pin | Status |
|---|---|---|
| `referral_core`, `referral_program` (tests), the program binary | logos-blockchain/uplink `master` @ `8021f33` | merged |
| LEZ crates (`lee_core`, `indexer_service_protocol`) | tag `v0.3.1`, as uplink pins them | released |
| `lez_core` | logos-execution-zone-module PR #69 @ `dde621a` (0.5.2): `send_generic_private_transaction_with_accounts` | open |
| `lez_indexer_module` | `87463b1` (1.2.1) | released |

Update the pins when the PRs merge.
