use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{transfer_checked, TransferChecked},
    token_interface::{Mint, TokenAccount, TokenInterface},
};

use crate::{Bank, User};

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        seeds = [mint.key().as_ref()],
        bump
    )]
    pub bank: Account<'info, Bank>,

    #[account(
        mut,
        seeds = [b"treasury", mint.key().as_ref()],
        bump
    )]
    pub bank_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [signer.key().as_ref()],
        bump
    )]
    pub user_account: Account<'info, User>,

    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = signer,
        associated_token::token_program = token_program
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info().key(),
            TransferChecked {
                authority: ctx.accounts.signer.to_account_info(),
                from: ctx.accounts.user_token_account.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.bank_token_account.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    let bank = &mut ctx.accounts.bank;

    let user_shares = if bank.total_deposit == 0 {
        amount
    } else {
        let deposit_ratio = amount.checked_div(bank.total_deposit).unwrap();
        bank.total_deposit_shared
            .checked_mul(deposit_ratio)
            .unwrap()
    };

    let user = &mut ctx.accounts.user_account;

    match ctx.accounts.mint.to_account_info().key() {
        key if key == user.usdc_address => {
            user.deposited_usdc += amount;
            user.deposited_usdc_shares += user_shares;
        }
        _ => {
            user.deposited_sol += amount;
            user.deposited_sol_shares += user_shares;
        }
    }

    bank.total_deposit += amount;
    bank.total_deposit_shared += user_shares;

    Ok(())
}
