//! End-to-end tests against a real SVM.
//!
//! These cover what only a runtime can prove: that account constraints actually reject the
//! transactions they are supposed to, that PDA derivation matches, and that tokens genuinely move.
//! The arithmetic itself is covered far more thoroughly in `logic.rs`, which needs no validator and
//! runs in microseconds.
//!
//! Mints and token accounts are written directly with `set_account` rather than built through the
//! token program's own instructions. The layouts are stable and documented, and doing it this way
//! keeps each test's setup to a few lines instead of a dozen transactions.

use anchor_lang::{AnchorSerialize, Discriminator, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use token_platform::state::VestingSchedule;

/// SPL Token program id.
const TOKEN_PROGRAM: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("HxRsEhSd2zBMTDDmQsiR4jA1ZGjdQVybjXrwHAedJUDJ");

const DAY: i64 = 24 * 60 * 60;
const YEAR: i64 = 365 * DAY;

/* -------------------------------------------------------------------- setup --- */

struct Harness {
    svm: LiteSVM,
    payer: Keypair,
    mint: Pubkey,
}

impl Harness {
    fn new() -> Self {
        let mut svm = LiteSVM::new();

        let so = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/deploy/token_platform.so");
        svm.add_program_from_file(PROGRAM_ID, so)
            .expect("build the program first: `anchor build`");

        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();

        let mint = Pubkey::new_unique();
        let mut harness = Self {svm, payer, mint};
        harness.write_mint(mint, 9);

        harness
    }

    /// Write an initialised SPL mint straight into the accounts database.
    ///
    /// The 82-byte layout is: mint_authority COption (4 + 32), supply (8), decimals (1),
    /// is_initialized (1), freeze_authority COption (4 + 32).
    fn write_mint(&mut self, mint: Pubkey, decimals: u8) {
        let mut data = vec![0u8; 82];

        data[0..4].copy_from_slice(&1u32.to_le_bytes()); // Some(authority)
        data[4..36].copy_from_slice(self.payer.pubkey().as_ref());
        data[36..44].copy_from_slice(&0u64.to_le_bytes()); // supply
        data[44] = decimals;
        data[45] = 1; // is_initialized

        self.svm
            .set_account(
                mint,
                Account {
                    lamports: 1_461_600,
                    data,
                    owner: TOKEN_PROGRAM,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Write an initialised SPL token account. The layout is 165 bytes.
    fn write_token_account(&mut self, address: Pubkey, owner: Pubkey, amount: u64) {
        let mut data = vec![0u8; 165];

        data[0..32].copy_from_slice(self.mint.as_ref());
        data[32..64].copy_from_slice(owner.as_ref());
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        data[108] = 1; // AccountState::Initialized

        self.svm
            .set_account(
                address,
                Account {
                    lamports: 2_039_280,
                    data,
                    owner: TOKEN_PROGRAM,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    fn token_balance(&self, address: &Pubkey) -> u64 {
        let account = self.svm.get_account(address).expect("account missing");
        u64::from_le_bytes(account.data[64..72].try_into().unwrap())
    }

    fn set_clock(&mut self, unix_timestamp: i64) {
        let mut clock: solana_clock::Clock = self.svm.get_sysvar();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    fn send(&mut self, instruction: Instruction, signers: &[&Keypair]) -> Result<(), String> {
        // Fresh blockhash per send. Two identical instructions under the same blockhash produce the
        // same transaction signature, and the runtime rejects the second as already processed,
        // which looks exactly like a program failure but is not one.
        self.svm.expire_blockhash();

        let message = Message::new(&[instruction], Some(&self.payer.pubkey()));
        let mut all_signers = vec![&self.payer];
        all_signers.extend_from_slice(signers);

        let tx = Transaction::new(&all_signers, message, self.svm.latest_blockhash());

        self.svm
            .send_transaction(tx)
            .map(|_| ())
            .map_err(|e| format!("{:?}", e.err))
    }
}

fn vesting_pda(beneficiary: &Pubkey, mint: &Pubkey, seed: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            VestingSchedule::SEED,
            beneficiary.as_ref(),
            mint.as_ref(),
            &seed.to_le_bytes(),
        ],
        &PROGRAM_ID,
    )
}

fn vault_pda(schedule: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VestingSchedule::VAULT_SEED, schedule.as_ref()], &PROGRAM_ID)
}

/* --------------------------------------------------------------------- tests --- */

#[test]
fn program_loads_and_exposes_its_id() {
    let harness = Harness::new();

    let account = harness.svm.get_account(&PROGRAM_ID).expect("program not loaded");
    assert!(account.executable, "program account should be executable");
}

/// The full vesting lifecycle against a real runtime: create, wait, release, and check the tokens
/// actually arrived in the beneficiary's account.
#[test]
fn vesting_creates_funds_and_releases() {
    let mut harness = Harness::new();
    harness.set_clock(1_000_000);

    let beneficiary = Keypair::new();
    let funding = Pubkey::new_unique();
    let beneficiary_ata = Pubkey::new_unique();

    harness.write_token_account(funding, harness.payer.pubkey(), 1_000_000);
    harness.write_token_account(beneficiary_ata, beneficiary.pubkey(), 0);

    let seed = 0u64;
    let (schedule, _) = vesting_pda(&beneficiary.pubkey(), &harness.mint, seed);
    let (vault, _) = vault_pda(&schedule);

    let create = Instruction {
        program_id: PROGRAM_ID,
        accounts: token_platform::accounts::CreateVesting {
            authority: harness.payer.pubkey(),
            beneficiary: beneficiary.pubkey(),
            schedule,
            vault,
            funding_account: funding,
            mint: harness.mint,
            token_program: TOKEN_PROGRAM,
            system_program: solana_pubkey::Pubkey::from_str_const("11111111111111111111111111111111"),
        }
        .to_account_metas(None),
        data: token_platform::instruction::CreateVesting {
            seed,
            amount: 1_000_000,
            start_ts: 1_000_000,
            cliff_seconds: YEAR,
            duration_seconds: 4 * YEAR,
            revocable: true,
        }
        .data(),
    };

    harness.send(create, &[]).expect("create_vesting failed");

    assert_eq!(harness.token_balance(&vault), 1_000_000, "grant funded up front");
    assert_eq!(harness.token_balance(&funding), 0, "pulled from the funder");

    // Before the cliff, nothing is releasable and the instruction rejects.
    let release = |harness: &Harness| Instruction {
        program_id: PROGRAM_ID,
        accounts: token_platform::accounts::ReleaseVesting {
            payer: harness.payer.pubkey(),
            schedule,
            vault,
            beneficiary_token_account: beneficiary_ata,
            mint: harness.mint,
            token_program: TOKEN_PROGRAM,
        }
        .to_account_metas(None),
        data: token_platform::instruction::ReleaseVesting {}.data(),
    };

    let instruction = release(&harness);
    assert!(
        harness.send(instruction, &[]).is_err(),
        "nothing should be releasable before the cliff"
    );

    // Half way through: exactly half should be releasable.
    harness.set_clock(1_000_000 + 2 * YEAR);

    let instruction = release(&harness);
    harness.send(instruction, &[]).expect("release failed");

    assert_eq!(
        harness.token_balance(&beneficiary_ata),
        500_000,
        "half the grant at the half way point"
    );
    assert_eq!(harness.token_balance(&vault), 500_000);
}

/// Releasing is permissionless, but the destination is constrained to the beneficiary's own
/// account. This is the constraint that makes leaving it open safe, and it can only be proven
/// against a runtime that enforces `token::authority`.
#[test]
fn a_release_cannot_be_redirected_to_an_attacker() {
    let mut harness = Harness::new();
    harness.set_clock(1_000_000);

    let beneficiary = Keypair::new();
    let attacker = Keypair::new();
    let funding = Pubkey::new_unique();
    let attacker_ata = Pubkey::new_unique();

    harness.write_token_account(funding, harness.payer.pubkey(), 1_000_000);
    harness.write_token_account(attacker_ata, attacker.pubkey(), 0);

    let seed = 0u64;
    let (schedule, _) = vesting_pda(&beneficiary.pubkey(), &harness.mint, seed);
    let (vault, _) = vault_pda(&schedule);

    let create = Instruction {
        program_id: PROGRAM_ID,
        accounts: token_platform::accounts::CreateVesting {
            authority: harness.payer.pubkey(),
            beneficiary: beneficiary.pubkey(),
            schedule,
            vault,
            funding_account: funding,
            mint: harness.mint,
            token_program: TOKEN_PROGRAM,
            system_program: Pubkey::from_str_const("11111111111111111111111111111111"),
        }
        .to_account_metas(None),
        data: token_platform::instruction::CreateVesting {
            seed,
            amount: 1_000_000,
            start_ts: 1_000_000,
            cliff_seconds: 0,
            duration_seconds: 4 * YEAR,
            revocable: false,
        }
        .data(),
    };

    harness.send(create, &[]).expect("create_vesting failed");
    harness.set_clock(1_000_000 + 2 * YEAR);

    // The attacker submits a perfectly well-formed release, pointing at their own token account.
    let redirect = Instruction {
        program_id: PROGRAM_ID,
        accounts: token_platform::accounts::ReleaseVesting {
            payer: attacker.pubkey(),
            schedule,
            vault,
            beneficiary_token_account: attacker_ata,
            mint: harness.mint,
            token_program: TOKEN_PROGRAM,
        }
        .to_account_metas(None),
        data: token_platform::instruction::ReleaseVesting {}.data(),
    };

    assert!(
        harness.send(redirect, &[&attacker]).is_err(),
        "the destination constraint must reject an attacker-owned account"
    );
    assert_eq!(harness.token_balance(&attacker_ata), 0, "nothing moved");
    assert_eq!(harness.token_balance(&vault), 1_000_000, "the grant is untouched");
}

/// The vesting PDA is derived from beneficiary, mint and seed. A schedule cannot be created at an
/// address that does not match those, which is what stops an attacker planting a schedule account
/// of their own choosing.
#[test]
fn a_mismatched_schedule_pda_is_rejected() {
    let mut harness = Harness::new();
    harness.set_clock(1_000_000);

    let beneficiary = Keypair::new();
    let funding = Pubkey::new_unique();
    harness.write_token_account(funding, harness.payer.pubkey(), 1_000_000);

    // A PDA derived for a *different* beneficiary than the one passed in the accounts.
    let someone_else = Keypair::new();
    let (wrong_schedule, _) = vesting_pda(&someone_else.pubkey(), &harness.mint, 0);
    let (vault, _) = vault_pda(&wrong_schedule);

    let create = Instruction {
        program_id: PROGRAM_ID,
        accounts: token_platform::accounts::CreateVesting {
            authority: harness.payer.pubkey(),
            beneficiary: beneficiary.pubkey(),
            schedule: wrong_schedule,
            vault,
            funding_account: funding,
            mint: harness.mint,
            token_program: TOKEN_PROGRAM,
            system_program: Pubkey::from_str_const("11111111111111111111111111111111"),
        }
        .to_account_metas(None),
        data: token_platform::instruction::CreateVesting {
            seed: 0,
            amount: 1_000,
            start_ts: 1_000_000,
            cliff_seconds: 0,
            duration_seconds: YEAR,
            revocable: false,
        }
        .data(),
    };

    assert!(
        harness.send(create, &[]).is_err(),
        "seed constraints must reject a PDA derived from a different beneficiary"
    );
}

/// Anchor writes an 8-byte discriminator at the head of every account it owns, and refuses to
/// deserialise an account whose discriminator belongs to a different type. That is what stops one
/// account type being substituted for another, which is a classic Solana vulnerability.
#[test]
fn account_discriminators_are_distinct() {
    use token_platform::state::{ClaimStatus, Distributor, LaunchConfig, StakeAccount, StakePool};

    let discriminators = [
        VestingSchedule::DISCRIMINATOR,
        Distributor::DISCRIMINATOR,
        ClaimStatus::DISCRIMINATOR,
        LaunchConfig::DISCRIMINATOR,
        StakePool::DISCRIMINATOR,
        StakeAccount::DISCRIMINATOR,
    ];

    for (i, a) in discriminators.iter().enumerate() {
        for (j, b) in discriminators.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "types {i} and {j} share a discriminator");
            }
        }
    }
}

/// Sanity check that instruction data serialises to something the program can parse: the
/// discriminator plus borsh-encoded arguments.
#[test]
fn instruction_data_carries_a_discriminator() {
    let data = token_platform::instruction::CreateVesting {
        seed: 7,
        amount: 1_000,
        start_ts: 0,
        cliff_seconds: 0,
        duration_seconds: 100,
        revocable: true,
    }
    .data();

    assert_eq!(
        &data[..8],
        token_platform::instruction::CreateVesting::DISCRIMINATOR,
        "instruction data starts with its discriminator"
    );

    // seed(8) + amount(8) + start(8) + cliff(8) + duration(8) + revocable(1) after the tag.
    assert_eq!(data.len(), 8 + 8 + 8 + 8 + 8 + 8 + 1);

    let mut buffer = Vec::new();
    7u64.serialize(&mut buffer).unwrap();
    assert_eq!(&data[8..16], &buffer[..], "first argument is the seed");
}
