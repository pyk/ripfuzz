//! `rvm.finding` cheatcode - record explicit findings.

use crate::evm::cheatcode::{
    calls::Vm, outcome, state::ExecutionState, state::ReportedFinding, state::Severity,
};

/// Handle `rvm.finding(Finding)` with full metadata.
pub fn handle(
    state: &mut ExecutionState,
    finding: Vm::Finding,
) -> Option<revm::interpreter::CallOutcome> {
    if finding.id.is_empty() {
        return Some(outcome::revert("rvm.finding: id must not be empty"));
    }
    let severity = decode_severity(finding.severity);
    let Some(severity) = severity else {
        return Some(outcome::revert("rvm.finding: invalid severity"));
    };
    state.findings.push(ReportedFinding {
        id: finding.id,
        severity,
        title: finding.title,
        description: finding.description,
    });
    Some(outcome::success())
}

/// Handle `rvm.finding(string id)` with defaults.
pub fn handle_simple(
    state: &mut ExecutionState,
    id: &str,
) -> Option<revm::interpreter::CallOutcome> {
    if id.is_empty() {
        return Some(outcome::revert("rvm.finding: id must not be empty"));
    }
    state.findings.push(ReportedFinding {
        id: id.to_owned(),
        severity: Severity::Medium,
        title: String::new(),
        description: String::new(),
    });
    Some(outcome::success())
}

fn decode_severity(value: Vm::Severity) -> Option<Severity> {
    match value {
        Vm::Severity::Info => Some(Severity::Info),
        Vm::Severity::Low => Some(Severity::Low),
        Vm::Severity::Medium => Some(Severity::Medium),
        Vm::Severity::High => Some(Severity::High),
        Vm::Severity::Critical => Some(Severity::Critical),
        // Solidity enums are exhaustive, but handle unknown as None
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::evm::cheatcode::calls::Vm;
    use crate::evm::cheatcode::state::{ExecutionState, Severity};

    use super::*;

    #[test]
    fn handle_finding_stores_full_metadata() {
        let mut state = ExecutionState::default();
        let finding = Vm::Finding {
            id: "INV-001".into(),
            severity: Vm::Severity::High,
            title: "bad state".into(),
            description: "details".into(),
        };
        let outcome = handle(&mut state, finding);
        assert!(outcome.is_some());
        assert!(outcome.unwrap().result.is_ok());
        assert_eq!(state.findings.len(), 1);
        assert_eq!(state.findings[0].id, "INV-001");
        assert_eq!(state.findings[0].severity, Severity::High);
        assert_eq!(state.findings[0].title, "bad state");
        assert_eq!(state.findings[0].description, "details");
    }

    #[test]
    fn handle_simple_uses_default_severity() {
        let mut state = ExecutionState::default();
        let outcome = handle_simple(&mut state, "SIMPLE");
        assert!(outcome.is_some());
        assert!(outcome.unwrap().result.is_ok());
        assert_eq!(state.findings.len(), 1);
        assert_eq!(state.findings[0].id, "SIMPLE");
        assert_eq!(state.findings[0].severity, Severity::Medium);
        assert!(state.findings[0].title.is_empty());
    }

    #[test]
    fn handle_empty_id_reverts() {
        let mut state = ExecutionState::default();
        let finding = Vm::Finding {
            id: "".into(),
            severity: Vm::Severity::Info,
            title: "".into(),
            description: "".into(),
        };
        let outcome = handle(&mut state, finding).unwrap();
        assert!(outcome.result.is_revert());
        assert!(state.findings.is_empty());
    }

    #[test]
    fn handle_simple_empty_id_reverts() {
        let mut state = ExecutionState::default();
        let outcome = handle_simple(&mut state, "").unwrap();
        assert!(outcome.result.is_revert());
        assert!(state.findings.is_empty());
    }

    #[test]
    fn severity_from_u8_roundtrips() {
        assert_eq!(Severity::from_u8(0), Some(Severity::Info));
        assert_eq!(Severity::from_u8(4), Some(Severity::Critical));
        assert_eq!(Severity::from_u8(5), None);
    }
}
