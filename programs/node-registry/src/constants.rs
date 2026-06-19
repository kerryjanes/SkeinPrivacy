/// PDA seed for the singleton [`crate::state::Registry`].
pub const REGISTRY_SEED: &[u8] = b"registry";
/// PDA seed for a [`crate::state::TreeShard`].
pub const TREE_SEED: &[u8] = b"tree";
/// PDA seed for a [`crate::state::NodeState`].
pub const NODE_SEED: &[u8] = b"node";

/// Node is live and eligible for traffic.
pub const STATUS_ACTIVE: u8 = 0;
/// Node is temporarily suspended (e.g. by reputation/governance, M3+).
pub const STATUS_SUSPENDED: u8 = 1;
/// Node has been deregistered (cNFT burned).
pub const STATUS_DEREGISTERED: u8 = 2;

/// Maximum allowed merkle tree depth (2^30 leaves).
pub const MAX_TREE_DEPTH: u32 = 30;
