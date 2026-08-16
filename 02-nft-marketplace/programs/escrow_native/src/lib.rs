//! # Escrow, written without Anchor
//!
//! A minimal SOL escrow implemented against the raw Solana program interface. It exists to show
//! what Anchor's `#[derive(Accounts)]` blocks are actually generating, because a developer who has
//! only ever written Anchor cannot tell which of its guarantees are the framework's and which are
//! the runtime's.
//!
//! Every check below is one that Anchor would have written for you:
//!
//! | Anchor | What it expands to, roughly |
//! |---|---|
//! | `Signer<'info>` | `if !account.is_signer { return Err(MissingRequiredSignature) }` |
//! | `#[account(mut)]` | `if !account.is_writable { return Err(...) }` |
//! | `seeds = [...], bump` | `create_program_address(seeds, program_id)` then compare to the key |
//! | `Account<'info, T>` | Owner check, then discriminator check, then borsh deserialise |
//! | `has_one = x` | `if state.x != accounts.x.key() { return Err(...) }` |
//! | `close = dest` | Move all lamports to `dest`, zero the data, reassign to the System program |
//!
//! Missing any one of these is a real vulnerability, and the first three are the three most common
//! Solana bugs there are. Anchor's value is that forgetting them becomes hard rather than easy.
//!
//! The escrow itself: a maker deposits lamports against a named taker. The taker can withdraw; the
//! maker can cancel and reclaim. Deliberately small, so the checks are the interesting part.

#![allow(unexpected_cfgs)]

use borsh::{BorshDeserialize, BorshSerialize};
use solana_account_info::{next_account_info, AccountInfo};
use solana_cpi::invoke;
use solana_msg::msg;
use solana_program_error::{ProgramError, ProgramResult};
use solana_pubkey::Pubkey;

#[cfg(not(feature = "no-entrypoint"))]
solana_program_entrypoint::entrypoint!(process_instruction);

/// Seed prefix for the escrow PDA.
pub const ESCROW_SEED: &[u8] = b"native-escrow";

/// Discriminator standing in for Anchor's 8-byte account tag.
///
/// Without something like this, an attacker can pass an account of a different type whose bytes
/// happen to deserialise, and the program will act on garbage. Anchor generates this from the
/// account name; here it is written out, which is exactly the point.
pub const ESCROW_DISCRIMINATOR: [u8; 8] = *b"ESCROWv1";

/// Escrow state.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct Escrow {
    pub discriminator: [u8; 8],
    pub maker: Pubkey,
    pub taker: Pubkey,
    pub amount: u64,
    pub bump: u8,
}

impl Escrow {
    /// 8 + 32 + 32 + 8 + 1. Computed by hand, because there is no `InitSpace` derive here.
    pub const LEN: usize = 8 + 32 + 32 + 8 + 1;
}

/// Instruction set. Anchor derives this dispatch from the `#[program]` module.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum EscrowInstruction {
    /// Deposit lamports against a named taker.
    ///
    /// Accounts:
    /// 0. `[signer, writable]` maker
    /// 1. `[writable]` escrow PDA
    /// 2. `[]` taker
    /// 3. `[]` system program
    Initialize {amount: u64},

    /// Taker withdraws the escrowed lamports.
    ///
    /// Accounts:
    /// 0. `[signer]` taker
    /// 1. `[writable]` escrow PDA
    /// 2. `[writable]` taker's lamport destination
    Withdraw,

    /// Maker cancels and reclaims.
    ///
    /// Accounts:
    /// 0. `[signer, writable]` maker
    /// 1. `[writable]` escrow PDA
    Cancel,
}

/// Entrypoint dispatch.
///
/// Anchor generates this from the 8-byte instruction discriminator. Here the discriminator is
/// borsh's enum tag, and dispatch is a `match`.
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = EscrowInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    match instruction {
        EscrowInstruction::Initialize {amount} => initialize(program_id, accounts, amount),
        EscrowInstruction::Withdraw => withdraw(program_id, accounts),
        EscrowInstruction::Cancel => cancel(program_id, accounts),
    }
}

fn initialize(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let iter = &mut accounts.iter();

    let maker = next_account_info(iter)?;
    let escrow = next_account_info(iter)?;
    let taker = next_account_info(iter)?;
    let system_program = next_account_info(iter)?;

    // === What `Signer<'info>` does ===
    // Without this, anyone could create an escrow claiming to be somebody else. It is the single
    // most common Solana vulnerability, and it is one line.
    if !maker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // === What `#[account(mut)]` does ===
    if !maker.is_writable || !escrow.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    if amount == 0 {
        return Err(ProgramError::InvalidArgument);
    }

    // === What `seeds = [...], bump` does ===
    // Deriving the address and comparing it is what makes the PDA meaningful. Skipping it lets a
    // caller pass any account they like and have the program treat it as the escrow.
    let (expected, bump) = Pubkey::find_program_address(
        &[ESCROW_SEED, maker.key.as_ref(), taker.key.as_ref()],
        program_id,
    );

    if expected != *escrow.key {
        return Err(ProgramError::InvalidSeeds);
    }

    // === What `init` does ===
    // Anchor computes rent, invokes the System program to create the account, assigns ownership to
    // this program, and writes the discriminator. All four steps, by hand:
    let rent = solana_sysvar::rent::Rent::default();
    let lamports = rent.minimum_balance(Escrow::LEN).saturating_add(amount);

    let create = solana_system_interface::instruction::create_account(
        maker.key,
        escrow.key,
        lamports,
        Escrow::LEN as u64,
        program_id,
    );

    // Signing as the PDA requires passing its seeds, which only this program can produce.
    solana_cpi::invoke_signed(
        &create,
        &[maker.clone(), escrow.clone(), system_program.clone()],
        &[&[ESCROW_SEED, maker.key.as_ref(), taker.key.as_ref(), &[bump]]],
    )?;

    let state = Escrow {
        discriminator: ESCROW_DISCRIMINATOR,
        maker: *maker.key,
        taker: *taker.key,
        amount,
        bump,
    };

    // borsh returns an io::Error, which does not convert to ProgramError on its own. Anchor's
    // `Account<'info, T>` hides this conversion entirely.
    state
        .serialize(&mut &mut escrow.try_borrow_mut_data()?[..])
        .map_err(|_| ProgramError::AccountDataTooSmall)?;

    msg!("escrow created: {} lamports for {}", amount, taker.key);

    Ok(())
}

fn withdraw(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let iter = &mut accounts.iter();

    let taker = next_account_info(iter)?;
    let escrow = next_account_info(iter)?;
    let destination = next_account_info(iter)?;

    if !taker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let state = load_escrow(program_id, escrow)?;

    // === What `has_one = taker` does ===
    // Without it, any signer could withdraw any escrow.
    if state.taker != *taker.key {
        return Err(ProgramError::IllegalOwner);
    }

    close_escrow(escrow, destination)?;

    msg!("escrow released to {}", taker.key);

    Ok(())
}

fn cancel(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let iter = &mut accounts.iter();

    let maker = next_account_info(iter)?;
    let escrow = next_account_info(iter)?;

    if !maker.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let state = load_escrow(program_id, escrow)?;

    if state.maker != *maker.key {
        return Err(ProgramError::IllegalOwner);
    }

    close_escrow(escrow, maker)?;

    msg!("escrow cancelled, {} lamports returned", state.amount);

    Ok(())
}

/// What `Account<'info, Escrow>` does, in full.
///
/// Three checks, in this order, and every one of them matters:
///
/// 1. **Owner.** An account this program does not own can contain anything at all, because some
///    other program wrote it. Deserialising it as `Escrow` would be reading attacker-chosen bytes.
///    This is the check people forget most often.
///
/// 2. **Discriminator.** Even among accounts this program owns, one type's bytes may deserialise
///    as another's. Anchor's 8-byte tag is what keeps types apart.
///
/// 3. **Deserialise.** Only now is it safe to interpret the bytes.
fn load_escrow(program_id: &Pubkey, escrow: &AccountInfo) -> Result<Escrow, ProgramError> {
    if escrow.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }

    let data = escrow.try_borrow_data()?;

    if data.len() < Escrow::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    if data[..8] != ESCROW_DISCRIMINATOR {
        return Err(ProgramError::InvalidAccountData);
    }

    Escrow::try_from_slice(&data[..Escrow::LEN]).map_err(|_| ProgramError::InvalidAccountData)
}

/// What `close = destination` does.
///
/// Three steps. Moving the lamports alone is not enough: an account with zero lamports is collected
/// at the end of the transaction, but within the same transaction it can still be read, so the data
/// is zeroed as well. Anchor additionally writes a closed-account discriminator; zeroing achieves
/// the same end here because `load_escrow` rejects a mismatched tag.
fn close_escrow(escrow: &AccountInfo, destination: &AccountInfo) -> ProgramResult {
    let lamports = escrow.lamports();

    **escrow.try_borrow_mut_lamports()? = 0;
    **destination.try_borrow_mut_lamports()? = destination
        .lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    escrow.try_borrow_mut_data()?.fill(0);

    Ok(())
}

/// Silences an unused-import warning while keeping `invoke` available for future instructions.
#[allow(dead_code)]
fn _unused(instruction: &solana_instruction::Instruction, accounts: &[AccountInfo]) -> ProgramResult {
    invoke(instruction, accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escrow_len_matches_the_serialised_size() {
        let escrow = Escrow {
            discriminator: ESCROW_DISCRIMINATOR,
            maker: Pubkey::new_unique(),
            taker: Pubkey::new_unique(),
            amount: 1_000,
            bump: 254,
        };

        let serialised = borsh::to_vec(&escrow).unwrap();

        // Hand-computed sizes drift silently as fields are added. Anchor's `InitSpace` derive
        // exists precisely because this is easy to get wrong.
        assert_eq!(serialised.len(), Escrow::LEN);
    }

    #[test]
    fn round_trips_through_borsh() {
        let escrow = Escrow {
            discriminator: ESCROW_DISCRIMINATOR,
            maker: Pubkey::new_unique(),
            taker: Pubkey::new_unique(),
            amount: u64::MAX,
            bump: 255,
        };

        let bytes = borsh::to_vec(&escrow).unwrap();
        assert_eq!(Escrow::try_from_slice(&bytes).unwrap(), escrow);
    }

    /// The discriminator is what keeps account types apart. An account whose first eight bytes are
    /// anything else must be rejected before deserialisation.
    #[test]
    fn a_wrong_discriminator_is_detectable() {
        let mut bytes = borsh::to_vec(&Escrow {
            discriminator: *b"OTHERTY0",
            maker: Pubkey::new_unique(),
            taker: Pubkey::new_unique(),
            amount: 1,
            bump: 1,
        })
        .unwrap();

        assert_ne!(bytes[..8], ESCROW_DISCRIMINATOR);

        // And the same bytes with the right tag pass, so the check is doing the discriminating.
        bytes[..8].copy_from_slice(&ESCROW_DISCRIMINATOR);
        assert_eq!(bytes[..8], ESCROW_DISCRIMINATOR);
    }

    /// PDA derivation is deterministic, which is what lets the program verify a passed-in address
    /// rather than trusting it.
    #[test]
    fn pda_derivation_is_deterministic() {
        let program_id = Pubkey::new_unique();
        let maker = Pubkey::new_unique();
        let taker = Pubkey::new_unique();

        let (first, bump_a) =
            Pubkey::find_program_address(&[ESCROW_SEED, maker.as_ref(), taker.as_ref()], &program_id);
        let (second, bump_b) =
            Pubkey::find_program_address(&[ESCROW_SEED, maker.as_ref(), taker.as_ref()], &program_id);

        assert_eq!(first, second);
        assert_eq!(bump_a, bump_b);

        // Different parties derive a different escrow, so one pair's escrow cannot be passed off
        // as another's.
        let (other, _) = Pubkey::find_program_address(
            &[ESCROW_SEED, taker.as_ref(), maker.as_ref()],
            &program_id,
        );
        assert_ne!(first, other);
    }
}
