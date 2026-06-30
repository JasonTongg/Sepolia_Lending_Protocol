use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Insufficient Fund")]
    InsufficientFunds,
    #[msg("Over Borrowable Amount")]
    OverBorrowableAmount,
    #[msg("Over Repay")]
    OverRepay,
}
