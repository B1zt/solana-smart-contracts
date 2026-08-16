# SPL Token Platform

Everything a Solana project needs to launch and distribute a token: a capped SPL mint whose
authority is a program-derived address, linear vesting with cliffs, a Merkle airdrop, and a staking
pool with streamed rewards.

One Anchor program in Rust, a TypeScript backend, and a Next.js frontend with wallet adapter.

---

## Written for Solana, not translated from Ethereum

The most common failure in Solana code from EVM developers is a faithful translation of Ethereum
patterns that the runtime does not reward. Four decisions here go the other way, and each is
explained where it appears in the code.

**PDAs replace privileged addresses.** The mint authority, every vault and every pool are
program-derived addresses. A PDA has no private key and can only sign through this program, so
authority is bounded by code rather than by key custody. There is no equivalent of "the owner key
was compromised".

**Account existence replaces boolean flags.** An airdrop claim is recorded by creating a PDA. The
runtime refuses to initialise the same address twice, so a replayed claim fails before the program
checks anything. Compare the EVM version in this portfolio, which packs a claim bitmap by hand:
here there is no flag to forget to set.

**Accumulators replace iteration.** Staking rewards use one running `reward_per_token` figure rather
than per-user bookkeeping. On Solana this is structural, not an optimisation: a transaction touches
a bounded set of accounts, so anything that loops over participants stops working once the pool gets
popular.

**Constraints replace defensive code.** Most of the security lives in `#[derive(Accounts)]`:
`has_one`, `seeds`, `token::authority` and `address` are all checked before a handler runs. The
three classic Solana vulnerabilities, a missing signer check, a substituted account and an
unvalidated PDA, are closed declaratively rather than with runtime `if` statements.

---

## What is actually interesting here

**The off-chain and on-chain Merkle trees are pinned to each other.** Both build from the same
fixed entry set and assert the same root, in
[`cross_check.rs`](programs/token_platform/tests/cross_check.rs) and
[`tree.test.ts`](backend/src/merkle/tree.test.ts). This matters more here than on EVM: Rust's
`to_le_bytes` is **little-endian** while every EVM codebase is big-endian, so a tree builder ported
across without noticing produces well-formed proofs that fail every claim, and the on-chain error
says only `InvalidProof`. A dedicated test asserts the endianness directly, and asserts that
big-endian really would differ so it cannot pass vacuously.

**Releasing vested tokens is permissionless but cannot be redirected.** `token::authority =
schedule.beneficiary` on the destination account is what makes that safe, so a project can pay the
fee for users who never claim without being able to move a single token elsewhere. There is a
litesvm test where an attacker submits a well-formed release into their own account and it fails.

**Revocation pays out what was already earned first.** Vested-but-unclaimed tokens go to the
beneficiary in the same instruction, and only the remainder returns to the authority. Skipping that
step would let an authority time a revocation to confiscate earned tokens.

**Transaction submission handles the two things that actually go wrong on Solana.** A blockhash is
valid for about a minute, and an expired transaction cannot be resent because the blockhash is
inside what was signed; it has to be rebuilt. And the base fee does no ordering work under
contention, so landing a transaction when the network is busy means attaching a compute unit price
derived from what recent transactions on the same accounts paid. See
[`transactions.ts`](backend/src/chain/transactions.ts).

**The indexer uses `getProgramAccounts` for state and signatures for history.** Solana has no
`eth_getLogs`, no block range filter and no topic index. What it has instead is better for current
state: one call returns every account the program owns, already at its current values, with no event
replay. History still requires walking signatures backwards, which is why the cursor stores a
signature rather than a block number.

---

## Layout

```
programs/token_platform/
  src/
    lib.rs             16 instructions across four modules
    state.rs           Account layouts, vesting curve, reward accumulator
    instructions/
      launch.rs        Mint authority handover, capped minting, one-way finish
      vesting.rs       Create, release, revoke, close and reclaim rent
      airdrop.rs       Merkle claims with per-claim PDAs, clawback after the deadline
      staking.rs       Accumulator rewards, cooldown unstaking, reward funding
  tests/
    logic.rs           22 unit tests: vesting curves, Merkle, accumulator maths
    integration.rs     6 litesvm tests: constraints, PDAs, real token movement
    cross_check.rs     3 tests pinning the Rust and TypeScript Merkle trees together

backend/            Merkle proof service, account indexer, priority-fee aware sender
frontend/           Airdrop claim, vesting dashboard, staking, wallet adapter
```

---

## Running it

```bash
# Toolchain
solana --version     # 3.x (Agave)
anchor --version     # 1.1.x

anchor build         # builds the program and generates the IDL
cargo test           # 31 tests
cargo clippy --all-targets   # clean

# Local validator
solana-test-validator
anchor deploy

cd backend  && cp .env.example .env && pnpm install && pnpm db:migrate && pnpm dev
cd frontend && cp .env.example .env.local && pnpm install && pnpm dev
```

---

## Tests

```bash
cargo test                                  # 31 tests
cargo test --test logic                     # 22 pure-logic tests, microseconds
cargo test --test integration               # 6 litesvm tests
cargo test --test cross_check               # 3 Rust/TypeScript agreement tests

cd backend && pnpm test                     # 14 TypeScript Merkle tests
```

Properties worth calling out:

- `vesting_is_monotonic` - vesting never decreases across the whole schedule
- `large_grants_do_not_overflow` - and asserts the naive u64 form really would have overflowed
- `an_internal_node_cannot_pass_as_a_leaf` - the reason leaves are double hashed
- `a_late_staker_earns_nothing_from_before_they_joined` - the accumulator's core property
- `a_release_cannot_be_redirected_to_an_attacker` - the constraint that makes releases open
- `a_mismatched_schedule_pda_is_rejected` - seed constraints actually bind
- `integers_are_encoded_little_endian` - the trap that silently breaks ported Merkle code

---

## Security notes

| Decision | Reason |
|---|---|
| Mint authority is a PDA | A PDA has no key to compromise; minting is bounded by code |
| Supply cap counts pre-existing supply | Otherwise a project pre-mints, then mints the cap again |
| `finish_minting` is one-way | Checked on every mint, so no later authority change reopens it |
| Vesting funded at creation | A schedule the vault cannot pay is a promise, not a commitment |
| One vault per grant | A shared pool makes one beneficiary's bug everyone's problem |
| Release destination is constrained | What makes a permissionless release safe |
| Revocation pays vested tokens first | Otherwise revocation becomes confiscation |
| Vesting maths runs in u128 | `total * elapsed` overflows u64 for realistic grants |
| Claim status is a PDA, not a flag | The runtime enforces single-claim; no flag to forget |
| Airdrop leaves bind index, claimant and amount | Each closes a distinct substitution attack |
| Leaves are double hashed | A 32-byte internal node cannot pass as a leaf |
| Clawback gated on the published deadline | Otherwise the authority drains when claims look slow |
| Proof length is bounded | Stops a caller burning compute with an enormous proof |
| Staking measures what arrived | A Token-2022 transfer fee delivers less than was sent |
| `init_if_needed` paired with an ownership check | The known Anchor footgun, closed explicitly |
| Cooldown capped at 30 days | An unbounded cooldown is indistinguishable from confiscation |
| Rewards pay at most the vault balance | A pool that runs dry winds down rather than bricking |
| Vesting duration capped at 10 years | A century-long schedule is a burn; burning is the honest way |

The upgrade authority should be a multisig or removed entirely before mainnet. An upgradeable
Solana program can be replaced wholesale by whoever holds that authority, which is a larger power
than any `onlyOwner` function.

This code has not been audited. It is a reference implementation.

---

## License

MIT
