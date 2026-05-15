use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use revm::{
    Database,
    context_interface::ContextTr,
    inspector::Inspector,
    interpreter::{
        CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome, Gas, InstructionResult,
        InterpreterResult,
    },
    primitives::{Address, Bytes},
};

/// Foundry cheatcode VM contract address.
pub(crate) const VM_ADDRESS: Address = Address::new([
    0x71, 0x09, 0x70, 0x9e, 0xcf, 0xa9, 0x1a, 0x80, 0x62, 0x6f, 0xf3, 0x98, 0x9d, 0x68, 0xf6, 0x7f,
    0x5b, 0x1d, 0xd1, 0x2d,
]);

/// Insert a dummy VM contract into the database so Solidity's
/// `extcodesize` check passes when a target calls Foundry cheatcodes.
pub(crate) fn insert_foundry_vm(db: &mut revm::database::InMemoryDB) {
    let vm_code = revm::bytecode::Bytecode::new_raw(revm::primitives::Bytes::from_static(&[0x00]));
    db.insert_account_info(
        VM_ADDRESS,
        revm::state::AccountInfo {
            balance: revm::primitives::U256::ZERO,
            nonce: 0,
            code_hash: vm_code.hash_slow(),
            code: Some(vm_code),
            account_id: None,
        },
    );
}

/// A single frame in a call trace.
#[derive(Debug, Clone)]
struct TraceFrame {
    kind: TraceKind,
    address: Address,
    input: Bytes,
    gas_used: u64,
    result: TraceResult,
    contract_name: Option<String>,
    func_name: Option<String>,
    decoded_args: Option<String>,
    decoded_return: Option<String>,
    scheme: Option<CallScheme>,
    /// Deployed address (set for CREATE frames).
    created_address: Option<Address>,
    /// Size of deployed code (set for successful CREATE frames).
    code_size: usize,
}

#[derive(Debug, Clone)]
enum TraceKind {
    Call {
        target: Address,
    },
    Create,
    /// Intercepted Foundry cheatcode call (hidden from trace output).
    VmCall,
}

#[derive(Debug, Clone)]
enum TraceResult {
    Success,
    Revert { reason: String },
    Halt { reason: String },
}

/// Node in a call trace tree.
#[derive(Debug)]
struct CallNode {
    frame: TraceFrame,
    children: Vec<CallNode>,
}

/// Inspector that records a call / create trace with contract and function names.
#[derive(Debug)]
pub struct CallTraceInspector {
    stack: Vec<CallNode>,
    roots: Vec<CallNode>,
    /// Maps initcode to (contract name, abi) for CREATE name resolution.
    initcode_map: HashMap<Bytes, (String, JsonAbi)>,
    /// Maps deployed address to contract name.
    address_names: HashMap<Address, String>,
    /// Maps deployed address to ABI for CALL function decoding.
    address_abis: HashMap<Address, JsonAbi>,
}

impl CallTraceInspector {
    /// Build a new inspector with a mapping of known initcodes to contract metadata.
    pub fn new(initcode_map: HashMap<Bytes, (String, JsonAbi)>) -> Self {
        Self {
            stack: Vec::new(),
            roots: Vec::new(),
            initcode_map,
            address_names: HashMap::new(),
            address_abis: HashMap::new(),
        }
    }

    /// Attach a human-readable label to an address, equivalent to Foundry's `vm.label`.
    pub fn label(&mut self, address: Address, name: String) {
        self.address_names.insert(address, name);
    }

    /// Attach a label **and** an ABI to an address so that later CALLs are decoded.
    pub fn label_with_abi(&mut self, address: Address, name: String, abi: JsonAbi) {
        self.address_names.insert(address, name);
        self.address_abis.insert(address, abi);
    }

    /// Decode and apply a Foundry-style cheatcode.
    fn handle_cheatcode(&mut self, input: &Bytes) {
        if input.len() < 4 {
            return;
        }
        let sel: [u8; 4] = input[..4].try_into().unwrap_or([0; 4]);

        // label(address, string) = 0xc657c718
        const LABEL_SELECTOR: [u8; 4] = [0xc6, 0x57, 0xc7, 0x18];
        if sel == LABEL_SELECTOR {
            let types = vec![DynSolType::Address, DynSolType::String];
            let tuple = DynSolType::Tuple(types);
            if let Ok(DynSolValue::Tuple(values)) = tuple.abi_decode_params(&input[4..])
                && let [DynSolValue::Address(addr), DynSolValue::String(name)] = values.as_slice()
            {
                self.address_names.insert(*addr, name.clone());
            }
        }
    }

    pub fn format(&self) -> String {
        let mut lines = Vec::new();
        for (i, root) in self.roots.iter().enumerate() {
            if i > 0 {
                lines.push(String::new());
            }
            format_node(root, "", true, &mut lines);
        }
        lines.join("\n")
    }
}

fn format_node(node: &CallNode, prefix: &str, is_last: bool, lines: &mut Vec<String>) {
    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };
    let call = format!(
        "[{}] {}{}",
        node.frame.gas_used,
        format_frame(&node.frame),
        format_scheme(&node.frame)
    );
    lines.push(format!("{}{}{}", prefix, connector, call));

    let child_prefix = if prefix.is_empty() {
        "  ".to_string()
    } else if is_last {
        format!("{}   ", prefix)
    } else {
        format!("{}│  ", prefix)
    };

    for child in &node.children {
        // Real children are never the "last" visual sibling because the
        // frame's return line always follows them.
        format_node(child, &child_prefix, false, lines);
    }

    let return_str = format_return(&node.frame);
    lines.push(format!("{}└─ {}", child_prefix, return_str));
}

fn format_frame(frame: &TraceFrame) -> String {
    match &frame.kind {
        TraceKind::Call { target } => {
            let func = match &frame.func_name {
                Some(name) => name.clone(),
                None => {
                    if frame.input.len() >= 4 {
                        format!("0x{}", hex::encode(&frame.input[..4]))
                    } else {
                        "???".to_string()
                    }
                }
            };
            let args = frame.decoded_args.as_deref().unwrap_or("()");
            match &frame.contract_name {
                Some(name) => format!("{}::{}{}", name, func, args),
                None => format!("{}::{}{}", format_address(*target), func, args),
            }
        }
        TraceKind::Create => match (&frame.contract_name, frame.created_address) {
            (Some(name), Some(addr)) => {
                format!("→ new {}@{:?}", name, addr)
            }
            (None, Some(addr)) => {
                format!("→ new 0x{:?}", addr)
            }
            (Some(name), None) => {
                format!("{}::constructor()", name)
            }
            (None, None) => {
                format!("{}::constructor()", format_address(frame.address))
            }
        },
        TraceKind::VmCall => {
            // VmCall nodes are discarded before formatting; unreachable.
            unreachable!("VmCall should never appear in formatted trace")
        }
    }
}

fn format_scheme(frame: &TraceFrame) -> String {
    match frame.scheme {
        Some(CallScheme::StaticCall) => " [staticcall]".to_string(),
        Some(CallScheme::DelegateCall) => " [delegatecall]".to_string(),
        Some(CallScheme::CallCode) => " [callcode]".to_string(),
        _ => String::new(),
    }
}

fn format_return(frame: &TraceFrame) -> String {
    match &frame.result {
        TraceResult::Success => {
            if let Some(ref ret) = frame.decoded_return {
                format!("← [Return] {}", ret)
            } else if matches!(frame.kind, TraceKind::Create) && frame.code_size > 0 {
                format!("← [Return] {} bytes of code", frame.code_size)
            } else {
                "← [Stop]".to_string()
            }
        }
        TraceResult::Revert { reason } => {
            format!("← [Revert] {}", reason)
        }
        TraceResult::Halt { reason } => {
            format!("← [Halt] {}", reason)
        }
    }
}

impl<CTX: ContextTr> Inspector<CTX> for CallTraceInspector {
    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let input = inputs.input.bytes_local(context.local());

        // Intercept Foundry cheatcode calls to the VM address.
        if inputs.target_address == VM_ADDRESS {
            self.handle_cheatcode(&input);
            // Push a dummy marker so call_end can balance the stack.
            self.stack.push(CallNode {
                frame: TraceFrame {
                    kind: TraceKind::VmCall,
                    address: VM_ADDRESS,
                    input: input.clone(),
                    gas_used: 0,
                    result: TraceResult::Success,
                    contract_name: Some("Vm".to_string()),
                    func_name: None,
                    decoded_args: None,
                    decoded_return: None,
                    scheme: Some(inputs.scheme),
                    created_address: None,
                    code_size: 0,
                },
                children: Vec::new(),
            });
            return Some(CallOutcome {
                result: InterpreterResult {
                    result: InstructionResult::Stop,
                    output: Bytes::new(),
                    gas: Gas::new(inputs.gas_limit),
                },
                memory_offset: inputs.return_memory_offset.clone(),
                was_precompile_called: false,
                precompile_call_logs: Vec::new(),
            });
        }

        // For DELEGATECALL / CALLCODE the code being executed lives at
        // `bytecode_address`, while `target_address` is the caller's
        // address (storage context).  Use the code address for name and
        // ABI lookup so we show the real function signature.
        let lookup_addr = if inputs.scheme.is_delegate_call() || inputs.scheme.is_call_code() {
            inputs.bytecode_address
        } else {
            inputs.target_address
        };
        let target = inputs.target_address;
        let contract_name = self.address_names.get(&lookup_addr).cloned();
        let func_name = if input.len() >= 4 {
            self.address_abis
                .get(&lookup_addr)
                .and_then(|abi| find_function_name(abi, &input[..4]))
        } else {
            None
        };
        let decoded_args = if input.len() >= 4 {
            self.address_abis
                .get(&lookup_addr)
                .and_then(|abi| decode_call_args(abi, &input, &self.address_names))
        } else {
            Some("()".to_string())
        };

        self.stack.push(CallNode {
            frame: TraceFrame {
                kind: TraceKind::Call { target },
                address: lookup_addr,
                input,
                gas_used: 0,
                result: TraceResult::Success,
                contract_name,
                func_name,
                decoded_args,
                decoded_return: None,
                scheme: Some(inputs.scheme),
                created_address: None,
                code_size: 0,
            },
            children: Vec::new(),
        });
        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        let mut node = match self.stack.pop() {
            Some(n) => n,
            None => return,
        };

        // Discard VM cheatcode calls — they never appear in the trace tree.
        if matches!(node.frame.kind, TraceKind::VmCall) {
            return;
        }

        let ir = &outcome.result;
        node.frame.gas_used = ir.gas.total_gas_spent();
        let abi = self.address_abis.get(&node.frame.address);
        node.frame.result = classify_result(ir.result, &ir.output, abi, &self.address_names);

        // Fallback: if the revert is still raw hex, try all known ABIs.
        if let TraceResult::Revert { reason } = &node.frame.result
            && reason.starts_with("0x")
        {
            for fallback_abi in self.address_abis.values() {
                if let Some(decoded) =
                    decode_custom_error(fallback_abi, &ir.output, &self.address_names)
                {
                    node.frame.result = TraceResult::Revert { reason: decoded };
                    break;
                }
            }
        }

        if ir.result.is_ok()
            && node.frame.input.len() >= 4
            && let Some(abi) = abi
        {
            node.frame.decoded_return =
                decode_return(abi, &node.frame.input[..4], &ir.output, &self.address_names);
        }

        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(node);
        } else {
            self.roots.push(node);
        }
    }

    fn create(&mut self, context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        let input = inputs.init_code().clone();

        let (contract_name, abi) = self.initcode_map.get(&input).cloned().unzip();

        // Pre-compute and register the created address so inner calls can
        // resolve the contract name before create_end fires.
        let caller = inputs.caller();
        let nonce = context
            .db_mut()
            .basic(caller)
            .ok()
            .flatten()
            .map(|info| info.nonce)
            .unwrap_or(0);
        // The address is computed from the nonce *before* it is incremented
        // for this create; try nonce.saturating_sub(1) first, fallback to nonce.
        let addr = inputs.created_address(nonce.saturating_sub(1));
        if let Some(ref name) = contract_name {
            self.address_names.insert(addr, name.clone());
        }
        if let Some(ref a) = abi {
            self.address_abis.insert(addr, a.clone());
        }

        self.stack.push(CallNode {
            frame: TraceFrame {
                kind: TraceKind::Create,
                address: inputs.caller(),
                input,
                gas_used: 0,
                result: TraceResult::Success,
                contract_name,
                func_name: None,
                decoded_args: None,
                decoded_return: None,
                scheme: None,
                created_address: Some(addr),
                code_size: 0,
            },
            children: Vec::new(),
        });
        None
    }

    fn create_end(
        &mut self,
        _context: &mut CTX,
        _inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        let mut node = match self.stack.pop() {
            Some(n) => n,
            None => return,
        };
        let ir = &outcome.result;
        node.frame.gas_used = ir.gas.total_gas_spent();
        let abi = self.initcode_map.get(&node.frame.input).map(|(_, abi)| abi);
        node.frame.result = classify_result(ir.result, &ir.output, abi, &self.address_names);

        // Fallback: if the revert is still raw hex, try all known ABIs.
        if let TraceResult::Revert { reason } = &node.frame.result
            && reason.starts_with("0x")
        {
            for fallback_abi in self.address_abis.values() {
                if let Some(decoded) =
                    decode_custom_error(fallback_abi, &ir.output, &self.address_names)
                {
                    node.frame.result = TraceResult::Revert { reason: decoded };
                    break;
                }
            }
        }

        // Record deployed code size for successful creates.
        if outcome.address.is_some() {
            node.frame.code_size = ir.output.len();
            node.frame.created_address = outcome.address;
        }

        // Register the deployed address so later CALLs can resolve names.
        if let Some(addr) = outcome.address {
            if let Some(name) = node.frame.contract_name.clone() {
                // Register mapping for address resolution
                self.address_names.insert(addr, name);
            }
            if let Some(ref abi) = self
                .initcode_map
                .get(&node.frame.input)
                .map(|(_, abi)| abi.clone())
            {
                self.address_abis.insert(addr, abi.clone());
            }
        }

        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(node);
        } else {
            self.roots.push(node);
        }
    }
}

fn format_address(addr: Address) -> String {
    let hex = format!("{:?}", addr);
    if hex.len() > 10 {
        format!("{}...{}", &hex[..6], &hex[hex.len() - 4..])
    } else {
        hex
    }
}

fn find_function_name(abi: &JsonAbi, selector: &[u8]) -> Option<String> {
    let sel: [u8; 4] = selector.try_into().ok()?;
    abi.functions()
        .find(|f| f.selector() == sel)
        .map(|f| f.name.clone())
}

fn decode_call_args(
    abi: &JsonAbi,
    data: &Bytes,
    labels: &HashMap<Address, String>,
) -> Option<String> {
    if data.len() < 4 {
        return Some("()".to_string());
    }
    let sel: [u8; 4] = data[..4].try_into().ok()?;
    let func = abi.functions().find(|f| f.selector() == sel)?;

    if func.inputs.is_empty() {
        return Some("()".to_string());
    }

    let types: Vec<DynSolType> = func
        .inputs
        .iter()
        .map(|p| p.selector_type().parse::<DynSolType>())
        .collect::<Result<_, _>>()
        .ok()?;

    let tuple = DynSolType::Tuple(types);
    let decoded = tuple.abi_decode_params(&data[4..]).ok()?;
    let values = match decoded {
        DynSolValue::Tuple(v) => v,
        other => vec![other],
    };

    let args = values
        .iter()
        .map(|v| format_value(v, labels))
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!("({})", args))
}

fn decode_return(
    abi: &JsonAbi,
    selector: &[u8],
    output: &Bytes,
    labels: &HashMap<Address, String>,
) -> Option<String> {
    let sel: [u8; 4] = selector.try_into().ok()?;
    let func = abi.functions().find(|f| f.selector() == sel)?;

    if func.outputs.is_empty() {
        return None;
    }

    let types: Vec<DynSolType> = func
        .outputs
        .iter()
        .map(|p| p.selector_type().parse::<DynSolType>())
        .collect::<Result<_, _>>()
        .ok()?;

    let tuple = DynSolType::Tuple(types);
    let decoded = tuple.abi_decode_params(output).ok()?;
    let values = match decoded {
        DynSolValue::Tuple(v) => v,
        other => vec![other],
    };

    let vals = values
        .iter()
        .map(|v| format_value(v, labels))
        .collect::<Vec<_>>()
        .join(", ");

    Some(vals)
}

fn format_value(v: &DynSolValue, labels: &HashMap<Address, String>) -> String {
    match v {
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::Int(i, _) => i.to_string(),
        DynSolValue::Uint(u, _) => u.to_string(),
        DynSolValue::Address(a) => labels
            .get(a)
            .map(|name| format!("{}: [{:?}]", name, a))
            .unwrap_or_else(|| format!("{:?}", a)),
        DynSolValue::String(s) => format!("\"{}\"", s),
        _ => format!("{:?}", v),
    }
}

/// Solidity `Error(string)` selector.
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

fn decode_solidity_error(output: &Bytes) -> Option<String> {
    if output.len() < 4 || output[..4] != ERROR_SELECTOR {
        return None;
    }
    let string_type = DynSolType::String;
    let decoded = string_type.abi_decode_params(&output[4..]).ok()?;
    match decoded {
        DynSolValue::String(s) => Some(s),
        _ => None,
    }
}

fn decode_custom_error(
    abi: &JsonAbi,
    output: &Bytes,
    labels: &HashMap<Address, String>,
) -> Option<String> {
    if output.len() < 4 {
        return None;
    }
    let sel: [u8; 4] = output[..4].try_into().ok()?;
    let error = abi.errors().find(|e| e.selector() == sel)?;

    if error.inputs.is_empty() {
        return Some(format!("{}()", error.name));
    }

    let types: Vec<DynSolType> = error
        .inputs
        .iter()
        .map(|p| p.selector_type().parse::<DynSolType>())
        .collect::<Result<_, _>>()
        .ok()?;

    let tuple = DynSolType::Tuple(types);
    let decoded = tuple.abi_decode_params(&output[4..]).ok()?;
    let values = match decoded {
        DynSolValue::Tuple(v) => v,
        other => vec![other],
    };

    let args = values
        .iter()
        .map(|v| format_value(v, labels))
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!("{}({})", error.name, args))
}

fn classify_result(
    result: InstructionResult,
    output: &Bytes,
    abi: Option<&JsonAbi>,
    labels: &HashMap<Address, String>,
) -> TraceResult {
    if result.is_revert() {
        let reason = abi
            .and_then(|a| decode_custom_error(a, output, labels))
            .or_else(|| decode_solidity_error(output))
            .unwrap_or_else(|| format!("0x{}", hex::encode(output)));
        TraceResult::Revert { reason }
    } else if result.is_error() {
        TraceResult::Halt {
            reason: format!("{:?}", result),
        }
    } else {
        TraceResult::Success
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::CallTraceInspector;
    use crate::contract::ContractBuilder;
    use revm::{
        MainBuilder, MainContext,
        context::{Context, TxEnv},
        database::InMemoryDB,
        inspector::InspectCommitEvm,
        primitives::{Address, Bytes, KECCAK_EMPTY, TxKind, U256},
        state::AccountInfo,
    };

    const CALLER: Address = Address::new([0xde; 20]);
    const GAS_LIMIT: u64 = 1_000_000;

    fn run_trace_case(name: &str, value: U256) -> String {
        let path = format!("src/{}.sol", name);
        let artifact = ContractBuilder::build(Path::new("fixtures/traces"), Path::new(&path))
            .unwrap_or_else(|e| panic!("failed to build {}: {}", name, e));

        let mut db = InMemoryDB::default();
        db.insert_account_info(
            CALLER,
            AccountInfo {
                balance: U256::from(1_000_000_000_000_000_000u128),
                nonce: 0,
                code_hash: KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
        super::insert_foundry_vm(&mut db);

        let initcode_map: HashMap<Bytes, (String, alloy_json_abi::JsonAbi)> = artifact
            .all_contracts
            .iter()
            .map(|(n, (ic, abi))| (ic.clone(), (n.clone(), abi.clone())))
            .collect();
        let inspector = CallTraceInspector::new(initcode_map);
        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let tx = TxEnv {
            caller: CALLER,
            kind: TxKind::Create,
            data: artifact.initcode.clone(),
            gas_limit: GAS_LIMIT,
            value,
            ..Default::default()
        };

        evm.inspect_tx_commit(tx).unwrap();
        evm.inspector.format()
    }

    macro_rules! trace_test {
        ($name:ident, $case:expr, $value:expr) => {
            #[test]
            fn $name() {
                let expected =
                    std::fs::read_to_string(format!("fixtures/traces/trace/{}.txt", $case))
                        .unwrap_or_else(|e| panic!("missing fixture for {}: {}", $case, e));
                let actual = run_trace_case($case, $value);
                assert_eq!(
                    actual.trim(),
                    expected.trim(),
                    "trace mismatch for {}",
                    $case
                );
            }
        };
    }

    trace_test!(simple_revert, "SimpleRevert", U256::ZERO);
    trace_test!(nested_revert, "NestedRevert", U256::ZERO);
    trace_test!(static_call_trace, "StaticCallTrace", U256::ZERO);
    trace_test!(return_value_trace, "ReturnValueTrace", U256::ZERO);
    trace_test!(payable_call_trace, "PayableCallTrace", U256::from(1000u128));
    trace_test!(multi_call_trace, "MultiCallTrace", U256::ZERO);
    trace_test!(deep_nesting_trace, "DeepNestingTrace", U256::ZERO);
    trace_test!(delegate_call_trace, "DelegateCallTrace", U256::ZERO);
    trace_test!(custom_error_trace, "CustomErrorTrace", U256::ZERO);
    trace_test!(helper_revert_trace, "HelperRevertTrace", U256::ZERO);

    #[test]
    fn label_call_trace() {
        let target_artifact = ContractBuilder::build(
            Path::new("fixtures/traces"),
            Path::new("src/ExternalTarget.sol"),
        )
        .unwrap();

        let trace_artifact = ContractBuilder::build(
            Path::new("fixtures/traces"),
            Path::new("src/LabelCallTrace.sol"),
        )
        .unwrap();

        let mut db = InMemoryDB::default();
        db.insert_account_info(
            CALLER,
            AccountInfo {
                balance: U256::from(1_000_000_000_000_000_000u128),
                nonce: 0,
                code_hash: KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
        super::insert_foundry_vm(&mut db);

        let external_addr = Address::new([0x11; 20]);
        db.insert_account_info(
            external_addr,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: target_artifact.runtime.hash_slow(),
                code: Some(target_artifact.runtime.clone()),
                account_id: None,
            },
        );

        let initcode_map: HashMap<Bytes, (String, alloy_json_abi::JsonAbi)> = trace_artifact
            .all_contracts
            .iter()
            .map(|(n, (ic, abi))| (ic.clone(), (n.clone(), abi.clone())))
            .collect();
        let mut inspector = CallTraceInspector::new(initcode_map);
        inspector.label_with_abi(
            external_addr,
            "ExternalTarget".to_string(),
            target_artifact.abi.clone(),
        );

        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let tx = TxEnv {
            caller: CALLER,
            kind: TxKind::Create,
            data: trace_artifact.initcode.clone(),
            gas_limit: GAS_LIMIT,
            value: U256::ZERO,
            ..Default::default()
        };

        evm.inspect_tx_commit(tx).unwrap();
        let actual = evm.inspector.format();

        let expected = std::fs::read_to_string("fixtures/traces/trace/LabelCallTrace.txt")
            .unwrap_or_else(|e| panic!("missing fixture: {}", e));
        assert_eq!(actual.trim(), expected.trim(), "trace mismatch");
    }

    #[test]
    fn vm_label_trace() {
        let target_artifact = ContractBuilder::build(
            Path::new("fixtures/traces"),
            Path::new("src/ExternalTarget.sol"),
        )
        .unwrap();

        let trace_artifact = ContractBuilder::build(
            Path::new("fixtures/traces"),
            Path::new("src/VmLabelTrace.sol"),
        )
        .unwrap();

        let mut db = InMemoryDB::default();
        db.insert_account_info(
            CALLER,
            AccountInfo {
                balance: U256::from(1_000_000_000_000_000_000u128),
                nonce: 0,
                code_hash: KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
        super::insert_foundry_vm(&mut db);

        let external_addr = Address::new([0x11; 20]);
        db.insert_account_info(
            external_addr,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: target_artifact.runtime.hash_slow(),
                code: Some(target_artifact.runtime.clone()),
                account_id: None,
            },
        );

        let initcode_map: HashMap<Bytes, (String, alloy_json_abi::JsonAbi)> = trace_artifact
            .all_contracts
            .iter()
            .map(|(n, (ic, abi))| (ic.clone(), (n.clone(), abi.clone())))
            .collect();
        let inspector = CallTraceInspector::new(initcode_map);
        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let tx = TxEnv {
            caller: CALLER,
            kind: TxKind::Create,
            data: trace_artifact.initcode.clone(),
            gas_limit: GAS_LIMIT,
            value: U256::ZERO,
            ..Default::default()
        };

        evm.inspect_tx_commit(tx).unwrap();
        let actual = evm.inspector.format();

        let expected = std::fs::read_to_string("fixtures/traces/trace/VmLabelTrace.txt")
            .unwrap_or_else(|e| panic!("missing fixture: {}", e));
        assert_eq!(actual.trim(), expected.trim(), "trace mismatch");
    }
}
