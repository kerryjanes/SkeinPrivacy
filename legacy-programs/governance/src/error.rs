use anchor_lang::prelude::*;

#[error_code]
pub enum GovError {
    #[msg("Caller is not authorized")]
    Unauthorized,
    #[msg("Proposal is not in the required state")]
    InvalidState,
    #[msg("Voting window is closed")]
    VotingClosed,
    #[msg("Voting window is still open")]
    VotingOngoing,
    #[msg("Quorum was not reached")]
    QuorumNotMet,
    #[msg("Approval threshold was not reached")]
    ThresholdNotMet,
    #[msg("Stake position unlocks before voting ends")]
    PositionUnlockedBeforeVoteEnds,
    #[msg("Stake position is not owned by the staking program")]
    InvalidPositionOwner,
    #[msg("Stake position operator does not match the signer")]
    PositionOperatorMismatch,
    #[msg("Execution timelock has not elapsed")]
    TimelockNotElapsed,
    #[msg("Transaction already executed")]
    AlreadyExecuted,
    #[msg("Proposer stake is below the minimum")]
    InsufficientProposalStake,
    #[msg("Payment split shares must sum to 10000 bps")]
    InvalidSplit,
    #[msg("Parameter out of range")]
    InvalidParam,
    #[msg("Too many accounts in the queued instruction")]
    TooManyAccounts,
    #[msg("Queued instruction data is too long")]
    DataTooLong,
    #[msg("Account list does not match the queued instruction")]
    AccountMismatch,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
