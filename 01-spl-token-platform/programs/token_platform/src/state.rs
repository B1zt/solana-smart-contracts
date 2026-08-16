use anchor_lang::prelude::*;

/// Global configuration for a token launch.
///
/// This account is also the mint authority. Handing the mint authority to a PDA rather than a
/// wallet is the single most important structural decision in a token launch: a PDA can only sign
/// through this program, so minting is bounded by whatever rules the program enforces rather than
/// by whoever holds a key.
#[account]
#[derive(InitSpace)]
pub struct LaunchConfig {
    /// Authority allowed to reconfigure the launch. Should be a multisig or a governance PDA.
    pub authority: Pubkey,

    /// Mint this config governs.
    pub mint: Pubkey,

    /// Hard supply cap, in base units. Immutable after initialisation.
    pub supply_cap: u64,

    /// Minted so far through this program.
    pub minted: u64,

    /// Once true, no further minting is possible regardless of authority.
    pub minting_finished: bool,

    /// Bump for the config PDA, cached so signing does not need a recomputation.
    pub bump: u8,

    /// Bump for the mint authority PDA.
    pub mint_authority_bump: u8,
}

impl LaunchConfig {
    pub const SEED: &'static [u8] = b"launch";
    pub const MINT_AUTHORITY_SEED: &'static [u8] = b"mint-authority";

    /// Tokens that may still be minted.
    pub fn remaining_mintable(&self) -> u64 {
        if self.minting_finished {
            return 0;
        }
        self.supply_cap.saturating_sub(self.minted)
    }
}

/// A linear vesting schedule with a cliff, funded up front.
///
/// One account per grant, each owning its own token vault. Per-grant vaults cost more rent than a
/// single shared pool, and they are worth it: a shared pool means one beneficiary's release path
/// touches balances that belong to everyone else, and any accounting bug becomes everyone's problem.
#[account]
#[derive(InitSpace)]
pub struct VestingSchedule {
    /// Who receives the tokens. Only this address is ever paid, whoever triggers the release.
    pub beneficiary: Pubkey,

    /// Authority that created the schedule and may revoke it, if revocable.
    pub authority: Pubkey,

    pub mint: Pubkey,

    /// Vault holding this grant. A PDA owned by the schedule.
    pub vault: Pubkey,

    /// Total granted over the whole schedule.
    pub total_amount: u64,

    /// Released so far. Never decreases.
    pub released_amount: u64,

    /// Vesting begins here. Nothing vests before it.
    pub start_ts: i64,

    /// Seconds after `start_ts` before anything is claimable.
    pub cliff_seconds: i64,

    /// Total length including the cliff.
    pub duration_seconds: i64,

    pub revocable: bool,

    /// Once revoked the schedule is frozen at whatever had vested.
    pub revoked: bool,

    /// Monotonic id, so one beneficiary can hold several grants.
    pub seed: u64,

    pub bump: u8,
}

impl VestingSchedule {
    pub const SEED: &'static [u8] = b"vesting";
    pub const VAULT_SEED: &'static [u8] = b"vesting-vault";

    /// Amount vested at `now`.
    ///
    /// Vesting is linear from `start_ts`, not from the end of the cliff, so crossing the cliff
    /// releases everything accrued during it as a single tranche. That is the convention every
    /// token unlock chart assumes, and diverging from it surprises people who read the chart.
    pub fn vested_amount(&self, now: i64) -> u64 {
        // A revoked schedule is frozen: `total_amount` was rewritten to the vested figure.
        if self.revoked {
            return self.total_amount;
        }

        if now < self.start_ts.saturating_add(self.cliff_seconds) {
            return 0;
        }

        if now >= self.start_ts.saturating_add(self.duration_seconds) {
            return self.total_amount;
        }

        let elapsed = now.saturating_sub(self.start_ts) as u128;
        let duration = self.duration_seconds as u128;

        // u128 throughout: `total_amount * elapsed` overflows u64 for any realistic grant size
        // multiplied by a multi-year duration in seconds.
        ((self.total_amount as u128).saturating_mul(elapsed) / duration) as u64
    }

    /// Vested and not yet claimed.
    pub fn releasable_amount(&self, now: i64) -> u64 {
        self.vested_amount(now).saturating_sub(self.released_amount)
    }
}

/// A Merkle airdrop.
///
/// Claim state lives in per-claim PDAs rather than a bitmap. Solana has no equivalent of Ethereum's
/// "cheap because the word is already warm" trick, and an account's existence is itself the flag:
/// the runtime refuses to initialise the same PDA twice, so a replayed claim fails without the
/// program checking anything.
#[account]
#[derive(InitSpace)]
pub struct Distributor {
    pub authority: Pubkey,
    pub mint: Pubkey,

    /// Vault holding the undistributed airdrop.
    pub vault: Pubkey,

    /// Root over `(index, claimant, amount)` leaves.
    pub merkle_root: [u8; 32],

    /// Total allocated across every leaf, for progress reporting.
    pub total_amount: u64,

    /// Claimed so far.
    pub claimed_amount: u64,

    /// Number of successful claims.
    pub claim_count: u64,

    /// After this, the authority may sweep whatever is unclaimed.
    ///
    /// Not hostile: without a deadline a meaningful fraction of any airdrop is stranded forever in
    /// wallets that will never claim. It is published up front and cannot be brought forward.
    pub claim_deadline: i64,

    pub bump: u8,
}

impl Distributor {
    pub const SEED: &'static [u8] = b"distributor";
    pub const VAULT_SEED: &'static [u8] = b"distributor-vault";
}

/// Marker proving one airdrop index has been claimed.
///
/// Deliberately almost empty. Its existence is the whole point: `init` on an already-initialised
/// PDA fails at the runtime level, so double claiming is impossible without the program comparing
/// anything at all.
#[account]
#[derive(InitSpace)]
pub struct ClaimStatus {
    pub claimant: Pubkey,
    pub amount: u64,
    pub claimed_at: i64,
    pub bump: u8,
}

impl ClaimStatus {
    pub const SEED: &'static [u8] = b"claim";
}

/// A staking pool paying rewards from a funded vault.
///
/// Rewards use the accumulator pattern: a single running `reward_per_token` figure, and each staker
/// stores the value it held when they last interacted. Their owed rewards are the difference,
/// multiplied by their stake. Nothing iterates over stakers, so the cost of a reward distribution
/// does not grow with the number of participants. On Solana that is not merely an optimisation:
/// there is no way to loop over an unbounded set of accounts within one transaction's limits.
#[account]
#[derive(InitSpace)]
pub struct StakePool {
    pub authority: Pubkey,

    /// Token being staked.
    pub stake_mint: Pubkey,

    /// Token paid as reward. May be the same as `stake_mint`.
    pub reward_mint: Pubkey,

    /// Vault holding staked principal.
    pub stake_vault: Pubkey,

    /// Vault holding undistributed rewards.
    pub reward_vault: Pubkey,

    /// Total currently staked.
    pub total_staked: u64,

    /// Rewards released per second while the schedule runs.
    pub reward_rate: u64,

    /// Rewards accrued per staked token, scaled by `ACC_PRECISION`.
    pub reward_per_token: u128,

    /// Last time the accumulator advanced.
    pub last_update_ts: i64,

    /// Emissions stop here.
    pub reward_end_ts: i64,

    /// Seconds a staker must wait after unstaking before withdrawing.
    pub cooldown_seconds: i64,

    /// Rewards committed to stakers but not yet withdrawn, so the pool cannot promise the same
    /// tokens twice or let the authority sweep what is owed.
    pub pending_rewards: u64,

    pub bump: u8,
}

impl StakePool {
    pub const SEED: &'static [u8] = b"pool";
    pub const STAKE_VAULT_SEED: &'static [u8] = b"stake-vault";
    pub const REWARD_VAULT_SEED: &'static [u8] = b"reward-vault";

    /// Scaling factor for `reward_per_token`.
    ///
    /// 1e12, not 1e18. The accumulator is multiplied by a staked balance that may itself be
    /// 1e9-scaled or larger, and u128 headroom disappears faster than it looks.
    pub const ACC_PRECISION: u128 = 1_000_000_000_000;

    /// Effective timestamp for accrual: emissions stop at `reward_end_ts`.
    pub fn accrual_ts(&self, now: i64) -> i64 {
        if now < self.reward_end_ts {
            now
        } else {
            self.reward_end_ts
        }
    }
}

/// One staker's position in a pool.
#[account]
#[derive(InitSpace)]
pub struct StakeAccount {
    pub owner: Pubkey,
    pub pool: Pubkey,

    /// Currently staked.
    pub amount: u64,

    /// Accumulator value when this position last settled. The offset that makes the pattern work.
    pub reward_debt: u128,

    /// Rewards earned and not yet withdrawn.
    pub pending_reward: u64,

    /// Amount unstaked and waiting out the cooldown.
    pub unstaking_amount: u64,

    /// When `unstaking_amount` becomes withdrawable.
    pub unstake_ready_ts: i64,

    pub bump: u8,
}

impl StakeAccount {
    pub const SEED: &'static [u8] = b"stake";
}
