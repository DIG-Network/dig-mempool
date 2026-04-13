//! Mempool error types.

use dig_clvm::Bytes32;

/// Errors returned by mempool operations.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MempoolError {
    #[error("bundle already seen: {0}")]
    AlreadySeen(Bytes32),

    #[error("duplicate spend of coin {0} within bundle")]
    DuplicateSpend(Bytes32),

    #[error("coin not found: {0}")]
    CoinNotFound(Bytes32),

    #[error("coin already spent: {0}")]
    CoinAlreadySpent(Bytes32),

    #[error("CLVM execution error: {0}")]
    ClvmError(String),

    #[error("invalid aggregate signature")]
    InvalidSignature,

    #[error("cost {cost} exceeds maximum {max}")]
    CostExceeded { cost: u64, max: u64 },

    #[error("negative fee: outputs ({output}) exceed inputs ({input})")]
    NegativeFee { input: u64, output: u64 },

    #[error("insufficient fee: required {required}, available {available}")]
    InsufficientFee { required: u64, available: u64 },

    #[error("fee too low for current mempool utilization")]
    FeeTooLow,

    #[error("conflicts with existing mempool item {0}")]
    Conflict(Bytes32),

    #[error("RBF rejected: must spend superset of conflicting bundle's coins")]
    RbfNotSuperset,

    #[error("RBF rejected: fee-per-cost not higher than existing")]
    RbfFpcNotHigher,

    #[error("RBF rejected: fee bump {provided} below minimum {required}")]
    RbfBumpTooLow { required: u64, provided: u64 },

    #[error("mempool full: cannot admit bundle")]
    MempoolFull,

    #[error("pending pool full")]
    PendingPoolFull,

    #[error("impossible timelock constraints")]
    ImpossibleTimelocks,

    #[error("bundle has expired")]
    Expired,

    #[error("conservation violation: input {input}, output {output}")]
    ConservationViolation { input: u64, output: u64 },

    #[error("dependency depth {depth} exceeds maximum {max}")]
    DependencyTooDeep { depth: u32, max: u32 },

    #[error("dependency cycle detected")]
    DependencyCycle,

    #[error("spend count {count} would exceed block limit {max}")]
    TooManySpends { count: usize, max: usize },

    #[error("admission policy rejected: {0}")]
    PolicyRejected(String),

    #[error("validation error: {0}")]
    ValidationError(String),
}
