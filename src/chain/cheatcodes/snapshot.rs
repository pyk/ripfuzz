//! Snapshot / revert cheatcodes.

use revm::{
    context_interface::ContextTr, database::InMemoryDB, interpreter::CallOutcome, primitives::U256,
};

use crate::chain::cheatcodes::{CheatcodeInspector, revert_outcome, success_bool_outcome};

/// `snapshot()` returns `uint256`.
pub const SNAPSHOT_SELECTOR: [u8; 4] = [0xb5, 0x61, 0x0e, 0xce];
/// `revertTo(uint256)` returns `bool`.
pub const REVERT_TO_SELECTOR: [u8; 4] = [0xb3, 0x08, 0xe4, 0x6f];

pub fn handle_snapshot<CTX: ContextTr<Db = InMemoryDB>>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
) -> Option<CallOutcome> {
    inspector.state.snapshots.push(ctx.db().clone());
    let id = U256::from(inspector.state.snapshots.len() - 1);
    Some(success_u256(id))
}

pub fn handle_revert_to<CTX: ContextTr<Db = InMemoryDB>>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let raw_id = super::decode_u256_arg(input)?;
    let id = match usize::try_from(raw_id) {
        Ok(v) => v,
        Err(_) => return Some(revert_outcome("invalid snapshot id")),
    };
    if id >= inspector.state.snapshots.len() {
        return Some(revert_outcome("invalid snapshot id"));
    }
    let db = inspector.state.snapshots[id].clone();
    *ctx.db_mut() = db;
    Some(success_bool_outcome(true))
}

fn success_u256(value: U256) -> CallOutcome {
    CallOutcome {
        result: revm::interpreter::InterpreterResult {
            result: revm::interpreter::InstructionResult::Return,
            output: revm::primitives::Bytes::from(value.to_be_bytes_vec()),
            gas: revm::interpreter::Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use revm::{
        Database, MainContext,
        context::Context,
        database::InMemoryDB,
        primitives::{Address, U256},
        state::AccountInfo,
    };

    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

    #[test]
    fn snapshot_increments_id() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let r1 = handle_snapshot(&mut inspector, &mut ctx).unwrap();
        let id1 = U256::from_be_slice(&r1.result.output);
        let r2 = handle_snapshot(&mut inspector, &mut ctx).unwrap();
        let id2 = U256::from_be_slice(&r2.result.output);
        assert_eq!(id1, U256::ZERO);
        assert_eq!(id2, U256::from(1));
    }

    #[test]
    fn revert_to_restores_state() {
        let addr = Address::new([0xab; 20]);
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            addr,
            AccountInfo {
                balance: U256::from(100),
                nonce: 0,
                code_hash: revm::primitives::KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(db);

        // Take snapshot
        handle_snapshot(&mut inspector, &mut ctx);

        // Mutate state
        let mut info = ctx.db_mut().basic(addr).unwrap().unwrap();
        info.balance = U256::from(200);
        ctx.db_mut().insert_account_info(addr, info);
        assert_eq!(
            ctx.db_mut().basic(addr).unwrap().unwrap().balance,
            U256::from(200)
        );

        // Revert to snapshot
        let mut input = vec![0u8; 4 + 32];
        input[0..4].copy_from_slice(&REVERT_TO_SELECTOR);
        let result = handle_revert_to(
            &mut inspector,
            &mut ctx,
            &revm::primitives::Bytes::from(input),
        );
        assert!(result.is_some());
        assert_eq!(
            ctx.db_mut().basic(addr).unwrap().unwrap().balance,
            U256::from(100)
        );
    }

    #[test]
    fn revert_to_invalid_id_reverts() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let mut input = vec![0u8; 4 + 32];
        input[0..4].copy_from_slice(&REVERT_TO_SELECTOR);
        input[4 + 31] = 99; // U256(99) big-endian
        let result = handle_revert_to(
            &mut inspector,
            &mut ctx,
            &revm::primitives::Bytes::from(input),
        );
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Revert
        );
    }
}
