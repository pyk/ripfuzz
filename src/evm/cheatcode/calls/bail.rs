//! `rvm.bail` cheatcode - report a broken invariant and abort the call.

use crate::evm::cheatcode::{calls::Vm, outcome, state::BrokenInvariant, state::ExecutionState};

/// Handle `rvm.bail(Invariant)` by recording the report and reverting the
/// call, so the harness replaces the `assert(false)` panic with a finding
/// that carries an id and description.
pub fn handle(
    state: &mut ExecutionState,
    invariant: Vm::Invariant,
) -> Option<revm::interpreter::CallOutcome> {
    if invariant.id.is_empty() {
        return Some(outcome::revert("rvm.bail: id must not be empty"));
    }
    state.broken_invariants.push(BrokenInvariant {
        id: invariant.id,
        description: invariant.description,
    });
    Some(outcome::revert("invariant broken"))
}

#[cfg(test)]
mod tests {
    use crate::evm::cheatcode::calls::Vm;
    use crate::evm::cheatcode::state::ExecutionState;

    use super::*;

    #[test]
    fn handle_bail_stores_report_and_reverts() {
        let mut state = ExecutionState::default();
        let invariant = Vm::Invariant {
            id: "INV-001".into(),
            description: "total exceeded 100".into(),
        };
        let outcome = handle(&mut state, invariant).unwrap();
        assert!(outcome.result.is_revert());
        assert_eq!(state.broken_invariants.len(), 1);
        assert_eq!(state.broken_invariants[0].id, "INV-001");
        assert_eq!(state.broken_invariants[0].description, "total exceeded 100");
    }

    #[test]
    fn handle_empty_id_reverts_without_report() {
        let mut state = ExecutionState::default();
        let invariant = Vm::Invariant {
            id: "".into(),
            description: "details".into(),
        };
        let outcome = handle(&mut state, invariant).unwrap();
        assert!(outcome.result.is_revert());
        assert!(state.broken_invariants.is_empty());
    }
}
