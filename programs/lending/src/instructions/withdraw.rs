use std::f64::consts::E;

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{transfer_checked, TransferChecked},
    token_interface::{Mint, TokenAccount, TokenInterface},
};

use crate::error::ErrorCode;
use crate::{Bank, User};

#[derive(Accounts)]
pub struct Withdraw<'info> {
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
        init_if_needed,
        payer = signer,
        associated_token::mint = mint,
        associated_token::authority = signer,
        associated_token::token_program = token_program
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
    let user = &mut ctx.accounts.user_account;
    let bank = &mut ctx.accounts.bank;

    let deposited_value: u64;
    if *ctx.accounts.mint.to_account_info().key == user.usdc_address {
        deposited_value = user.deposited_usdc;
    } else {
        deposited_value = user.deposited_sol;
    }

    let time_dif = user.last_updated - Clock::get()?.unix_timestamp;

    bank.total_deposit =
        (bank.total_deposit as f64 + E.powf(bank.interest_rate as f64 * time_dif as f64)) as u64;

    let value_per_share = bank.total_deposit as f64 / bank.total_deposit_shared as f64;

    let user_value = deposited_value as f64 / value_per_share;

    if user_value < amount as f64 {
        return Err(ErrorCode::InsufficientFunds.into());
    }

    let mint_key = ctx.accounts.mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury".as_ref(),
        mint_key.as_ref(),
        &[ctx.bumps.bank_token_account],
    ]];

    transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info().key(),
            TransferChecked {
                authority: ctx.accounts.bank.to_account_info(),
                from: ctx.accounts.bank_token_account.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.user_token_account.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    let bank = &mut ctx.accounts.bank;
    let shares_to_remove =
        (amount as f64 / bank.total_deposit as f64) * bank.total_deposit_shared as f64;

    let user = &mut ctx.accounts.user_account;
    if ctx.accounts.mint.to_account_info().key() == user.usdc_address {
        user.deposited_usdc -= amount;
        user.deposited_usdc_shares -= shares_to_remove as u64;
    } else {
        user.deposited_sol -= amount;
        user.deposited_sol_shares -= shares_to_remove as u64;
    }

    bank.total_deposit -= amount;
    bank.total_deposit_shared -= shares_to_remove as u64;
    Ok(())
}
