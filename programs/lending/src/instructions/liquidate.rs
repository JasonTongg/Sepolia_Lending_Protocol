use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{transfer_checked, TransferChecked},
    token_interface::{Mint, TokenAccount, TokenInterface},
};
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};

use crate::error::ErrorCode;
use crate::{Bank, User, SOL_USD_FEED_ID, USDC_USD_FEED_ID};
use super::borrow::calculate_accrued_interest;

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,

    pub price_update: Box<Account<'info, PriceUpdateV2>>,

    pub collateral_mint: Box<InterfaceAccount<'info, Mint>>,

    pub borrowed_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        seeds = [collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_bank: Box<Account<'info, Bank>>,

    #[account(
        mut,
        seeds = [borrowed_mint.key().as_ref()],
        bump
    )]
    pub borrowed_bank: Box<Account<'info, Bank>>,

    #[account(
        mut,
        seeds = [b"treasury", collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_bank_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"treasury", borrowed_mint.key().as_ref()],
        bump
    )]
    pub borrowed_bank_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [liquidator.key().as_ref()],
        bump
    )]
    pub user_account: Box<Account<'info, User>>,

    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = collateral_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_collateral_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = borrowed_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_borrowed_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

pub fn process_liquidate(ctx: Context<Liquidate>) -> Result<()> {
    let collateral_bank = &mut ctx.accounts.collateral_bank;
    let borrowed_bank = &mut ctx.accounts.borrowed_bank;
    let user = &mut ctx.accounts.user_account;

    let price_update = &mut ctx.accounts.price_update;

    let sol_feed_id = get_feed_id_from_hex(SOL_USD_FEED_ID);
    let usdc_feed_id = get_feed_id_from_hex(USDC_USD_FEED_ID);

    let sol_price = price_update.get_price_no_older_than(&Clock::get()?, 60, &sol_feed_id?)?;
    let usdc_price = price_update.get_price_no_older_than(&Clock::get()?, 60, &usdc_feed_id?)?;

    let total_collateral: u64;
    let total_borrowed: u64;

    match ctx.accounts.collateral_mint.to_account_info().key() {
        key if key == user.usdc_address => {
            let new_usdc = calculate_accrued_interest(
                user.deposited_usdc,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = usdc_price.price as u64 * new_usdc;
            let new_sol = calculate_accrued_interest(
                user.borrowed_sol,
                borrowed_bank.interest_rate,
                user.last_updated_borrowed,
            )?;
            total_borrowed = sol_price.price as u64 * new_sol;
        }
        _ => {
            let new_sol = calculate_accrued_interest(
                user.deposited_sol,
                collateral_bank.interest_rate,
                user.last_updated,
            )?;
            total_collateral = sol_price.price as u64 * new_sol;
            let new_usdc = calculate_accrued_interest(
                user.borrowed_usdc,
                borrowed_bank.interest_rate,
                user.last_updated_borrowed,
            )?;
            total_borrowed = usdc_price.price as u64 * new_usdc;
        }
    }

    let health_factor = ((total_collateral as f64 * collateral_bank.liquidation_threashold as f64)
        / total_borrowed as f64) as f64;

    if health_factor >= 1.0 {
        return Err(ErrorCode::NotUnderCollateralized.into());
    }

    let liquidation_amount = total_borrowed
        .checked_mul(borrowed_bank.liquidation_close_factor)
        .unwrap();

    transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info().key(),
            TransferChecked {
                authority: ctx.accounts.liquidator.to_account_info(),
                from: ctx
                    .accounts
                    .liquidator_borrowed_token_account
                    .to_account_info(),
                mint: ctx.accounts.borrowed_mint.to_account_info(),
                to: ctx.accounts.borrowed_bank_token_account.to_account_info(),
            },
        ),
        liquidation_amount,
        ctx.accounts.borrowed_mint.decimals,
    )?;

    let liquidator_amount = liquidation_amount
        .checked_mul(borrowed_bank.liquidation_bonus)
        .unwrap();

    let binding = ctx.accounts.collateral_mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        b"treasury".as_ref(),
        binding.as_ref(),
        &[ctx.bumps.collateral_bank_token_account],
    ]];

    transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info().key(),
            TransferChecked {
                authority: ctx.accounts.collateral_bank.to_account_info(),
                from: ctx.accounts.collateral_bank_token_account.to_account_info(),
                mint: ctx.accounts.collateral_mint.to_account_info(),
                to: ctx
                    .accounts
                    .liquidator_collateral_token_account
                    .to_account_info(),
            },
            signer_seeds,
        ),
        liquidator_amount,
        ctx.accounts.collateral_mint.decimals,
    )?;

    Ok(())
}

