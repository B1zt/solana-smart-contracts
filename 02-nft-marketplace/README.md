# NFT Marketplace

Collection minting with Merkle allowlist phases, and a marketplace where listed NFTs are held in
program-owned escrow.

Two Anchor programs in Rust, a TypeScript backend that indexes on-chain state and speaks the Digital
Asset Standard, and a Next.js frontend with wallet adapter.

---

## Written for Solana, not translated from Ethereum

**State is spread across accounts, not packed into one.** Mint phases and per-wallet mint counts are
separate PDAs rather than fields on the collection. A Solana account is fixed-size at creation, so an
inline `Vec<Phase>` would pay rent for its maximum length forever, and every mint in the drop would
contend on writing one account. Splitting them lets unrelated mints proceed in parallel, which is the
entire reason the account model looks the way it does. The EVM twin of this project in the portfolio
does the opposite, because on the EVM it should.

**Account existence replaces boolean flags.** A listing is an account. Delisting closes it. There is
no `isActive` field on-chain to forget to clear, and no state where a listing says it is active while
the escrow is empty. The indexer keeps an `isActive` column, but that is a view for the API rather
than the source of truth.

**Constraints replace defensive code.** Most of the security lives in `#[derive(Accounts)]`:
`has_one`, `seeds`, `address` and `token::authority` are checked before a handler runs. The three
classic Solana vulnerabilities, a missing signer check, a substituted account and an unvalidated PDA,
are closed declaratively.

**Royalties are a convention, not a guarantee.** Solana has no protocol-level royalty enforcement.
This marketplace reads the collection's royalty setting and pays it on every sale; a different
marketplace can simply not. That is a property of the chain rather than a gap in this program, and
saying so is more useful to a creator than implying they are protected.

---

## What is actually interesting here

**A second program, written without Anchor, showing what Anchor generates.**
[`escrow_native`](programs/escrow_native/src/lib.rs) is a minimal SOL escrow implemented against the
raw program interface, with a table mapping each Anchor attribute to the code it replaces: `Signer`
to an `is_signer` check, `seeds`/`bump` to `create_program_address` and a comparison,
`Account<'info, T>` to an owner check plus a discriminator check plus a borsh decode, `close` to
moving lamports, zeroing data and reassigning the owner. A developer who has only ever written Anchor
cannot tell which guarantees are the framework's and which are the runtime's. This is that answer,
in 350 lines.

**The off-chain and on-chain allowlists are pinned to each other.** Both build the same five-entry
fixture and assert the same root, in
[`cross_check.rs`](programs/nft_marketplace/tests/cross_check.rs) and
[`allowlist.test.ts`](backend/src/merkle/allowlist.test.ts). When a tree builder and a verifier
disagree the failure is silent and total: the API serves well-formed proofs and every mint fails with
`InvalidProof`. Rust's `to_le_bytes` is little-endian while every EVM codebase is big-endian, so this
is exactly the drift a port introduces, and a dedicated test pins the encoding directly and asserts
that big-endian genuinely differs, so it cannot pass vacuously.

**One Merkle root can express per-wallet tiers.** The leaf is
`keccak(keccak(wallet || allowance_le))`, so the allowance is bound into the proof. A wallet cannot
inflate its own allocation, and the same phase can grant one wallet 1 mint and another 5 without a
second root. The leaf is double-hashed so a 32-byte internal node cannot be presented as a leaf.

**Listed NFTs actually leave the seller's wallet.** The token moves into a program-owned escrow
account, so a seller cannot sell the same NFT elsewhere while a purchase is in flight, and a buy
cannot land against an empty wallet. The alternative, an approval the seller can revoke, is the
common design and the common source of failed purchases.

**Holdings are read through DAS rather than by scanning token accounts.** The obvious approach,
enumerate the wallet's token accounts and keep the ones with supply one, misses compressed NFTs
entirely, and compressed NFTs are the reason anyone picks Solana for a large collection. A compressed
NFT has no account at all: it is a leaf in a Merkle tree with only the root on-chain. The trade-off is
that transfers need a proof fetched from an indexer.
[`das.ts`](backend/src/chain/das.ts) handles that, and reports a non-DAS endpoint clearly instead of
failing opaquely, because `api.mainnet-beta.solana.com` does not implement DAS and that is the most
common surprise when wiring this up.

**Lamports move out of PDAs by direct balance adjustment.** A PDA holding data cannot be the source
of a System program transfer, because the System program refuses to move lamports out of an account
it does not own. Adjusting both balances directly is the supported way and works only because this
program owns the source account. `move_lamports` in
[`trading.rs`](programs/nft_marketplace/src/instructions/trading.rs) is where that lives, with the
reasoning next to it.

**Floor prices are recomputed rather than maintained incrementally.** Incremental is faster and
drifts permanently the moment one update is missed, and a wrong floor price is the most visible
possible bug on a marketplace front page.

---

## Layout

```
programs/
  nft_marketplace/     Anchor program: collections, phases, listings, offers
    src/state.rs       Collection, MintPhase, MintReceipt, Listing, Offer, Marketplace
    src/instructions/minting.rs   collection setup, phases, Merkle-gated mint
    src/instructions/trading.rs   list, buy, delist, offer, accept
    tests/logic.rs        Merkle verification and the payment split
    tests/cross_check.rs  the same root the TypeScript tests assert
  escrow_native/       the same escrow idea without Anchor, as a teaching artefact
backend/
  src/chain/das.ts     Digital Asset Standard client, compression cost comparison
  src/indexer/         getProgramAccounts with discriminator filters
  src/merkle/          allowlist builder, the TypeScript half of the cross-check
  src/routes.ts        collections, listings, activity, allowlist proofs, wallet assets
frontend/
  src/app/mint         phase timeline with live eligibility
  src/app/collections  listings, activity, compressed-asset badges
  src/app/portfolio    DAS-backed holdings
```

## Tests

20 Rust tests and 12 TypeScript tests.

```bash
cargo test --workspace                 # programs
cd backend && pnpm vitest run          # merkle and cross-check
```

## Running it

```bash
docker compose up -d                   # postgres on 5437

cd backend
cp .env.example .env                   # set DAS_RPC_URL for compressed NFTs
pnpm install && pnpm prisma migrate dev && pnpm dev

cd ../frontend
cp .env.example .env.local
pnpm install && pnpm dev               # http://localhost:3000
```

Building the programs needs the Solana toolchain:

```bash
anchor build                           # or: cargo build-sbf
```

## Known limits

The mint page builds and validates the transaction path but does not submit; wiring the final
`sendTransaction` needs a deployed program ID and a funded wallet, and a portfolio project that
prompts for a real signature is worse than one that does not.

Compressed minting itself is not implemented. The DAS integration reads compressed assets and the
cost comparison is real, but issuing them requires Bubblegum and a tree the project pays for. Reading
them is the part that changes how the application is built.
