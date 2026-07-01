use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_lang::prelude::Pubkey;
use lending::{Bank, User};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

fn token_program() -> Pubkey {
    TOKEN_PROGRAM_ID.parse().unwrap()
}
fn system_program() -> Pubkey {
    Pubkey::default()
}
// ATA program ID used by anchor-lang 1.x and litesvm
fn ata_program() -> Pubkey {
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL".parse().unwrap()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
    let bytes = include_bytes!("../../../target/deploy/lending.so");
    svm.add_program(lending::ID, bytes);
    (svm, payer)
}

fn send(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) {
    let bh = svm.latest_blockhash();
    let payer = signers[0].pubkey();
    let msg = Message::new_with_blockhash(ixs, Some(&payer), &bh);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx).expect("transaction failed");
}

fn try_send(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) -> bool {
    let bh = svm.latest_blockhash();
    let payer = signers[0].pubkey();
    let msg = Message::new_with_blockhash(ixs, Some(&payer), &bh);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx).is_ok()
}

/// Pubkey → litesvm's Address type (both are 32-byte wrappers)
fn to_addr(pk: &Pubkey) -> Address {
    Address::from(pk.to_bytes())
}

/// Create a new SPL Token mint using InitializeMint2 (tag=20, no rent sysvar)
fn create_mint(svm: &mut LiteSVM, payer: &Keypair, decimals: u8) -> Keypair {
    let mint = Keypair::new();
    let tok = token_program();
    let rent = svm.minimum_balance_for_rent_exemption(82);

    // System CreateAccount
    let mut create_data = vec![0u8, 0, 0, 0]; // CreateAccount tag (u32)
    create_data.extend_from_slice(&rent.to_le_bytes());
    create_data.extend_from_slice(&82u64.to_le_bytes());
    create_data.extend_from_slice(tok.as_ref());
    let create_ix = Instruction {
        program_id: system_program(),
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(mint.pubkey(), true),
        ],
        data: create_data,
    };

    // InitializeMint2 (tag=20): decimals, mint_authority, freeze_authority=None
    let mut init_data = vec![20u8, decimals];
    init_data.extend_from_slice(payer.pubkey().as_ref());
    init_data.push(0); // COption::None for freeze authority
    let init_ix = Instruction {
        program_id: tok,
        accounts: vec![AccountMeta::new(mint.pubkey(), false)],
        data: init_data,
    };

    send(svm, &[create_ix, init_ix], &[payer, &mint]);
    mint
}

/// Derive ATA address (uses standard mainnet ATA program ID for derivation)
fn get_ata(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program().as_ref(), mint.as_ref()],
        &ata_program(),
    )
    .0
}

/// Raw SPL Token account bytes (165 bytes, Initialized state, given amount)
fn token_account_data(mint: &Pubkey, owner: &Pubkey, amount: u64) -> Vec<u8> {
    let mut data = vec![0u8; 165];
    data[0..32].copy_from_slice(mint.as_ref());    // mint
    data[32..64].copy_from_slice(owner.as_ref());  // owner
    data[64..72].copy_from_slice(&amount.to_le_bytes()); // amount
    // delegate: COption::None → [0,0,0,0] + [0; 32] already zero
    data[108] = 1; // state: Initialized
    // is_native: None, delegated_amount: 0, close_authority: None → already zero
    data
}

/// Create a token account at the ATA address directly via set_account (no ATA program needed)
fn create_token_account(svm: &mut LiteSVM, wallet: &Pubkey, mint: &Pubkey, amount: u64) -> Pubkey {
    let ata = get_ata(wallet, mint);
    let rent = svm.minimum_balance_for_rent_exemption(165);
    svm.set_account(
        to_addr(&ata),
        Account {
            lamports: rent,
            data: token_account_data(mint, wallet, amount),
            owner: token_program(),
            executable: false,
            rent_epoch: u64::MAX,
        },
    )
    .unwrap();
    ata
}

/// Update the token account balance directly (simulates mint_to without CPI)
fn set_token_balance(svm: &mut LiteSVM, ata: &Pubkey, mint: &Pubkey, owner: &Pubkey, amount: u64) {
    let existing = svm.get_account(&to_addr(ata)).unwrap();
    svm.set_account(
        to_addr(ata),
        Account {
            lamports: existing.lamports,
            data: token_account_data(mint, owner, amount),
            owner: token_program(),
            executable: false,
            rent_epoch: u64::MAX,
        },
    )
    .unwrap();
}

// PDA helpers
fn bank_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[mint.as_ref()], &lending::ID).0
}
fn bank_token_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"treasury", mint.as_ref()], &lending::ID).0
}
fn user_pda(signer: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[signer.as_ref()], &lending::ID).0
}

fn read_bank(svm: &LiteSVM, mint: &Pubkey) -> Bank {
    let data = svm.get_account(&to_addr(&bank_pda(mint))).unwrap().data;
    Bank::try_deserialize(&mut &data[..]).unwrap()
}

fn read_user(svm: &LiteSVM, signer: &Pubkey) -> User {
    let data = svm.get_account(&to_addr(&user_pda(signer))).unwrap().data;
    User::try_deserialize(&mut &data[..]).unwrap()
}

// ── Reusable setup helpers ────────────────────────────────────────────────────

fn init_bank(svm: &mut LiteSVM, payer: &Keypair, mint: &Pubkey) {
    send(
        svm,
        &[Instruction {
            program_id: lending::ID,
            accounts: lending::accounts::InitBank {
                signer: payer.pubkey(),
                mint: *mint,
                bank: bank_pda(mint),
                bank_token_account: bank_token_pda(mint),
                system_program: system_program(),
                token_program: token_program(),
            }
            .to_account_metas(None),
            data: lending::instruction::InitBank {
                liquidation_threshold: 80,
                max_ltv: 75,
            }
            .data(),
        }],
        &[payer],
    );
}

fn init_user(svm: &mut LiteSVM, signer: &Keypair, usdc_mint: &Pubkey) {
    send(
        svm,
        &[Instruction {
            program_id: lending::ID,
            accounts: lending::accounts::InitUser {
                signer: signer.pubkey(),
                user_account: user_pda(&signer.pubkey()),
                system_program: system_program(),
            }
            .to_account_metas(None),
            data: lending::instruction::InitUser {
                usdc_address: *usdc_mint,
            }
            .data(),
        }],
        &[signer],
    );
}

fn deposit(svm: &mut LiteSVM, signer: &Keypair, mint: &Pubkey, amount: u64) {
    let ata = get_ata(&signer.pubkey(), mint);
    send(
        svm,
        &[Instruction {
            program_id: lending::ID,
            accounts: lending::accounts::Deposit {
                signer: signer.pubkey(),
                mint: *mint,
                bank: bank_pda(mint),
                bank_token_account: bank_token_pda(mint),
                user_account: user_pda(&signer.pubkey()),
                user_token_account: ata,
                system_program: system_program(),
                token_program: token_program(),
                associated_token_program: ata_program(),
            }
            .to_account_metas(None),
            data: lending::instruction::Deposit { amount }.data(),
        }],
        &[signer],
    );
}

fn withdraw(svm: &mut LiteSVM, signer: &Keypair, mint: &Pubkey, amount: u64) {
    let ata = get_ata(&signer.pubkey(), mint);
    send(
        svm,
        &[Instruction {
            program_id: lending::ID,
            accounts: lending::accounts::Withdraw {
                signer: signer.pubkey(),
                mint: *mint,
                bank: bank_pda(mint),
                bank_token_account: bank_token_pda(mint),
                user_account: user_pda(&signer.pubkey()),
                user_token_account: ata,
                token_program: token_program(),
                system_program: system_program(),
                associated_token_program: ata_program(),
            }
            .to_account_metas(None),
            data: lending::instruction::Withdraw { amount }.data(),
        }],
        &[signer],
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_init_bank() {
    let (mut svm, payer) = setup();
    let mint = create_mint(&mut svm, &payer, 6);

    init_bank(&mut svm, &payer, &mint.pubkey());

    let bank = read_bank(&svm, &mint.pubkey());
    assert_eq!(bank.authority, payer.pubkey());
    assert_eq!(bank.mint_address, mint.pubkey());
    assert_eq!(bank.liquidation_threashold, 80);
    assert_eq!(bank.max_ltv, 75);
    assert_eq!(bank.total_deposit, 0);
    assert_eq!(bank.total_borrowed, 0);
}

#[test]
fn test_init_user() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);

    init_user(&mut svm, &payer, &usdc_mint.pubkey());

    let user = read_user(&svm, &payer.pubkey());
    assert_eq!(user.owner, payer.pubkey());
    assert_eq!(user.usdc_address, usdc_mint.pubkey());
    assert_eq!(user.deposited_usdc, 0);
    assert_eq!(user.deposited_sol, 0);
    assert_eq!(user.borrowed_usdc, 0);
    assert_eq!(user.borrowed_sol, 0);
}

#[test]
fn test_init_bank_sol_and_usdc_are_independent() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);
    let sol_mint = create_mint(&mut svm, &payer, 9);

    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_bank(&mut svm, &payer, &sol_mint.pubkey());

    let usdc_bank = read_bank(&svm, &usdc_mint.pubkey());
    let sol_bank = read_bank(&svm, &sol_mint.pubkey());
    assert_eq!(usdc_bank.mint_address, usdc_mint.pubkey());
    assert_eq!(sol_bank.mint_address, sol_mint.pubkey());
    assert_ne!(bank_pda(&usdc_mint.pubkey()), bank_pda(&sol_mint.pubkey()));
}

#[test]
fn test_deposit_usdc_first_deposit() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);

    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey());

    let deposit_amount: u64 = 1_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &usdc_mint.pubkey(), deposit_amount);
    deposit(&mut svm, &payer, &usdc_mint.pubkey(), deposit_amount);

    let bank = read_bank(&svm, &usdc_mint.pubkey());
    assert_eq!(bank.total_deposit, deposit_amount);
    assert_eq!(bank.total_deposit_shared, deposit_amount);

    let user = read_user(&svm, &payer.pubkey());
    assert_eq!(user.deposited_usdc, deposit_amount);
    assert_eq!(user.deposited_usdc_shares, deposit_amount);
    assert_eq!(user.deposited_sol, 0);
}

#[test]
fn test_deposit_multiple_users_get_equal_shares() {
    let (mut svm, payer) = setup();
    let user2 = Keypair::new();
    svm.airdrop(&user2.pubkey(), 10_000_000_000).unwrap();

    let usdc_mint = create_mint(&mut svm, &payer, 6);
    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &user2, &usdc_mint.pubkey());

    let amount: u64 = 1_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &usdc_mint.pubkey(), amount);
    create_token_account(&mut svm, &user2.pubkey(), &usdc_mint.pubkey(), amount);

    deposit(&mut svm, &payer, &usdc_mint.pubkey(), amount);
    deposit(&mut svm, &user2, &usdc_mint.pubkey(), amount);

    let bank = read_bank(&svm, &usdc_mint.pubkey());
    assert_eq!(bank.total_deposit, amount * 2);

    let u1 = read_user(&svm, &payer.pubkey());
    let u2 = read_user(&svm, &user2.pubkey());
    // Both deposited equal amounts into an equal pool → equal shares
    assert_eq!(u1.deposited_usdc_shares, u2.deposited_usdc_shares);
}

#[test]
fn test_deposit_sol_token() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);
    let sol_mint = create_mint(&mut svm, &payer, 9);

    init_bank(&mut svm, &payer, &sol_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey()); // usdc_address = usdc_mint

    let deposit_amount: u64 = 5_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &sol_mint.pubkey(), deposit_amount);
    deposit(&mut svm, &payer, &sol_mint.pubkey(), deposit_amount);

    let user = read_user(&svm, &payer.pubkey());
    // sol_mint != usdc_address → goes into deposited_sol
    assert_eq!(user.deposited_sol, deposit_amount);
    assert_eq!(user.deposited_usdc, 0);
}

#[test]
fn test_withdraw_partial() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);

    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey());

    let deposit_amount: u64 = 1_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &usdc_mint.pubkey(), deposit_amount);
    deposit(&mut svm, &payer, &usdc_mint.pubkey(), deposit_amount);

    let withdraw_amount: u64 = 400_000_000;
    withdraw(&mut svm, &payer, &usdc_mint.pubkey(), withdraw_amount);

    let bank = read_bank(&svm, &usdc_mint.pubkey());
    assert_eq!(bank.total_deposit, deposit_amount - withdraw_amount);

    let user = read_user(&svm, &payer.pubkey());
    assert_eq!(user.deposited_usdc, deposit_amount - withdraw_amount);
}

#[test]
fn test_withdraw_full() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);

    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey());

    let amount: u64 = 1_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &usdc_mint.pubkey(), amount);
    deposit(&mut svm, &payer, &usdc_mint.pubkey(), amount);
    withdraw(&mut svm, &payer, &usdc_mint.pubkey(), amount);

    let bank = read_bank(&svm, &usdc_mint.pubkey());
    assert_eq!(bank.total_deposit, 0);

    let user = read_user(&svm, &payer.pubkey());
    assert_eq!(user.deposited_usdc, 0);
}

#[test]
fn test_withdraw_more_than_deposited_fails() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);

    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey());

    let amount: u64 = 1_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &usdc_mint.pubkey(), amount);
    deposit(&mut svm, &payer, &usdc_mint.pubkey(), amount);

    let ata = get_ata(&payer.pubkey(), &usdc_mint.pubkey());
    let withdraw_ix = Instruction {
        program_id: lending::ID,
        accounts: lending::accounts::Withdraw {
            signer: payer.pubkey(),
            mint: usdc_mint.pubkey(),
            bank: bank_pda(&usdc_mint.pubkey()),
            bank_token_account: bank_token_pda(&usdc_mint.pubkey()),
            user_account: user_pda(&payer.pubkey()),
            user_token_account: ata,
            token_program: token_program(),
            system_program: system_program(),
            associated_token_program: ata_program(),
        }
        .to_account_metas(None),
        data: lending::instruction::Withdraw {
            amount: amount * 2, // try to withdraw 2× deposited
        }
        .data(),
    };

    assert!(
        !try_send(&mut svm, &[withdraw_ix], &[&payer]),
        "withdrawing more than deposited must fail"
    );
}

#[test]
fn test_deposit_then_withdraw_bank_state_consistent() {
    let (mut svm, payer) = setup();
    let usdc_mint = create_mint(&mut svm, &payer, 6);

    init_bank(&mut svm, &payer, &usdc_mint.pubkey());
    init_user(&mut svm, &payer, &usdc_mint.pubkey());

    let amount: u64 = 2_000_000_000;
    create_token_account(&mut svm, &payer.pubkey(), &usdc_mint.pubkey(), amount);

    deposit(&mut svm, &payer, &usdc_mint.pubkey(), amount);
    let bank_after_deposit = read_bank(&svm, &usdc_mint.pubkey());

    withdraw(&mut svm, &payer, &usdc_mint.pubkey(), amount / 2);
    let bank_after_withdraw = read_bank(&svm, &usdc_mint.pubkey());

    // shares should decrease proportionally
    assert!(bank_after_withdraw.total_deposit < bank_after_deposit.total_deposit);
    assert!(bank_after_withdraw.total_deposit_shared < bank_after_deposit.total_deposit_shared);
    assert_eq!(bank_after_withdraw.total_deposit, amount / 2);
}
