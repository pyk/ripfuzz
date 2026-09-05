//! External function inspection for ripfuzz projects.
//!
//! [`ExternalFunctionsInspector`] compiles a contract file (or reuses a
//! cached compilation) and reports every externally callable function with
//! its source location, visibility, mutability, and modifiers.
//!
//! ```rust
//! use ripfuzz::config::Config;
//! use ripfuzz::harness::HarnessId;
//! use ripfuzz::inspectors::ExternalFunctionsInspector;
//!
//! let root = std::path::Path::new(".");
//! let config = Config::new().with_root(root).load("ripfuzz.toml")?;
//! let target = HarnessId::try_from("src/Voter.sol:Voter")?;
//! // let report = ExternalFunctionsInspector::new(root, config).inspect(&target)?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use solc::StandardJSONOutput;
use solc::abi::{Component, Function as AbiFunction, Item, Param, StateMutability};
use solc::ast::{
    ContractDefinition, ContractDefinitionNode, FunctionDefinition, FunctionKind, SourceUnitNode,
    VariableDeclaration, Visibility,
};

use crate::config::Config;
use crate::harness::HarnessId;
use crate::inspectors::CompiledTarget;

/// Source location of a resolved function declaration.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    /// File path relative to the project root.
    pub file: String,

    /// 1-based line number of the declaration.
    pub line: usize,
}

impl std::fmt::Display for SourceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// Metadata about a single externally callable function.
#[derive(Debug, Clone)]
pub struct ExternalFunctionInfo {
    /// Function name, e.g. `allocate`.
    pub name: String,

    /// Display signature, e.g. `allocate(uint256,Allocation[])`.
    pub signature: String,

    /// Hex-encoded 4-byte selector from the compiled method identifiers,
    /// e.g. `0xf02e634e`. Receive and fallback carry no selector.
    pub selector: Option<String>,

    /// Resolved source location, if known.
    pub source: Option<SourceInfo>,

    /// Solidity visibility of the resolved declaration.
    pub visibility: Visibility,

    /// State mutability from the compiled ABI.
    pub mutability: StateMutability,

    /// Modifier names, e.g. `["onlyOwner", "nonReentrant"]`.
    pub modifiers: Vec<String>,
}

/// The report produced by [`ExternalFunctionsInspector`].
#[derive(Debug)]
pub struct ExternalFunctionsOutput {
    /// Name of the inspected contract.
    pub contract_name: String,

    /// Source file of the inspected contract, relative to the project root.
    pub source_file: String,

    /// Nonpayable and payable functions.
    pub mutable: Vec<ExternalFunctionInfo>,

    /// View and pure functions.
    pub view: Vec<ExternalFunctionInfo>,

    /// Well-known token receiver callbacks.
    pub callback: Vec<ExternalFunctionInfo>,

    /// Receive and fallback functions.
    pub special: Vec<ExternalFunctionInfo>,
}

impl std::fmt::Display for ExternalFunctionsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 1. Header.
        writeln!(f, "# {} External Functions", self.contract_name)?;
        writeln!(f)?;
        writeln!(f, "- Contract: `{}`", self.contract_name)?;
        writeln!(f, "- File: `{}`", self.source_file)?;
        writeln!(f)?;

        // 2. Summary over every section.
        let total = self.mutable.len() + self.view.len() + self.callback.len() + self.special.len();
        writeln!(f, "## Summary")?;
        writeln!(f)?;
        writeln!(f, "- {total} externally callable functions")?;
        writeln!(f, "- {} mutable functions", self.mutable.len())?;
        writeln!(f, "- {} view functions", self.view.len())?;
        writeln!(f, "- {} callback functions", self.callback.len())?;
        writeln!(f, "- {} special functions", self.special.len())?;
        writeln!(f)?;

        // 3. Sections. Empty sections stay hidden because the summary
        //    already reports their counts. A section separates from any
        //    preceding content with a blank line.
        let mut first = true;
        write_section(f, &mut first, "Mutable Functions", &self.mutable)?;
        write_section(f, &mut first, "View Functions", &self.view)?;
        write_section(f, &mut first, "Callback Functions", &self.callback)?;
        write_section(f, &mut first, "Special Functions", &self.special)?;

        Ok(())
    }
}

/// Writes one titled section of the report as a markdown table.
fn write_section(
    f: &mut std::fmt::Formatter<'_>,
    first: &mut bool,
    title: &str,
    functions: &[ExternalFunctionInfo],
) -> std::fmt::Result {
    if functions.is_empty() {
        return Ok(());
    }
    if *first {
        *first = false;
    } else {
        writeln!(f)?;
    }
    writeln!(f, "## {title}")?;
    writeln!(f)?;
    let rows: Vec<Vec<String>> = functions
        .iter()
        .enumerate()
        .map(|(index, function)| {
            vec![
                (index + 1).to_string(),
                format!("`{}`", function.name),
                function
                    .selector
                    .as_ref()
                    .map(|selector| format!("`{selector}`"))
                    .unwrap_or_default(),
                format!("`{}`", mutability_label(&function.mutability)),
                modifiers_cell(&function.modifiers),
                source_cell(&function.source),
            ]
        })
        .collect();
    write_table(
        f,
        &["#", "Name", "Selector", "Mutability", "Modifiers", "Source"],
        &rows,
    )
}

/// Backtick-joined modifier cell for one function.
fn modifiers_cell(modifiers: &[String]) -> String {
    if modifiers.is_empty() {
        return "`none`".to_owned();
    }
    modifiers
        .iter()
        .map(|modifier| format!("`{modifier}`"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// Backticked source cell, or plain `unknown` when unresolved.
fn source_cell(source: &Option<SourceInfo>) -> String {
    match source {
        Some(source) => format!("`{source}`"),
        None => "unknown".to_owned(),
    }
}

/// Writes a padded markdown table. The first column is right-aligned, the
/// rest left-aligned.
fn write_table(
    f: &mut std::fmt::Formatter<'_>,
    headers: &[&str],
    rows: &[Vec<String>],
) -> std::fmt::Result {
    // 1. Column widths from the header and every row.
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }

    // 2. Header row and the alignment row below it. Alignment cells keep at
    //    least two dashes so every markdown renderer accepts them.
    let header: Vec<String> = headers.iter().map(|header| header.to_string()).collect();
    let separator: Vec<String> = (0..headers.len())
        .map(|index| {
            let dashes = "-".repeat(widths[index].saturating_sub(1).max(2));
            if index == 0 {
                format!("{dashes}:")
            } else {
                format!(":{dashes}")
            }
        })
        .collect();
    write_row(f, &header, &widths)?;
    write_row(f, &separator, &widths)?;

    // 3. One data row per function.
    for row in rows {
        write_row(f, row, &widths)?;
    }
    Ok(())
}

/// Writes one padded table row.
fn write_row(
    f: &mut std::fmt::Formatter<'_>,
    cells: &[String],
    widths: &[usize],
) -> std::fmt::Result {
    write!(f, "|")?;
    for (index, cell) in cells.iter().enumerate() {
        let width = widths[index];
        if index == 0 {
            write!(f, " {cell:>width$} |")?;
        } else {
            write!(f, " {cell:<width$} |")?;
        }
    }
    writeln!(f)
}

/// Inspects the external functions of a single contract.
///
/// The inspector compiles the target through the shared solc pipeline, so a
/// cached compilation keyed by the standard JSON input hash skips solc
/// entirely on repeated runs.
pub struct ExternalFunctionsInspector {
    root: PathBuf,
    config: Config,
}

impl ExternalFunctionsInspector {
    /// Creates an inspector for a project root and its loaded config.
    pub fn new(root: impl AsRef<Path>, config: Config) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            config,
        }
    }

    /// Inspects the external functions of `target`.
    pub fn inspect(&self, target: &HarnessId) -> Result<ExternalFunctionsOutput> {
        // 1. Compile the target or reuse a cached compilation.
        let compiled = CompiledTarget::compile(&self.root, &self.config, target)?;

        // 2. Extract the target contract ABI and method selectors.
        let contract_output = compiled.contract(target)?;
        let abi = contract_output.abi.as_ref().with_context(|| {
            format!(
                "contract `{}` has no ABI in the compilation output",
                target.name
            )
        })?;
        let selectors = contract_output
            .evm
            .as_ref()
            .and_then(|evm| evm.method_identifiers.clone())
            .unwrap_or_default();

        // 3. Index the compiled ASTs and classify every ABI item.
        let index = FunctionIndex::build(&compiled.output, &compiled.sources);
        let mut mutable = Vec::new();
        let mut view = Vec::new();
        let mut callback = Vec::new();
        let mut special = Vec::new();

        for item in &abi.items {
            match item {
                Item::Function(function) => {
                    let abi_key = abi_param_key(function);
                    let (source, visibility, modifiers) =
                        match index.resolve(&target.name, &function.name, &abi_key) {
                            Some(resolved) => (
                                Some(SourceInfo {
                                    file: resolved.file,
                                    line: resolved.line,
                                }),
                                resolved.visibility,
                                resolved.modifiers,
                            ),
                            None => (None, Visibility::External, Vec::new()),
                        };
                    let info = ExternalFunctionInfo {
                        // checkrs: allow(clone_in_loops)
                        name: function.name.clone(),
                        signature: display_signature(function),
                        selector: selectors.get(&canonical_signature(function)).cloned(),
                        source,
                        visibility,
                        mutability: function.state_mutability.clone(), // checkrs: allow(clone_in_loops)
                        modifiers,
                    };
                    if is_callback(&function.name) {
                        callback.push(info);
                    } else if matches!(
                        function.state_mutability,
                        StateMutability::View | StateMutability::Pure
                    ) {
                        view.push(info);
                    } else {
                        mutable.push(info);
                    }
                }
                Item::Receive(_) => {
                    special.push(ExternalFunctionInfo {
                        name: "receive".to_owned(),
                        signature: "receive()".to_owned(),
                        selector: None,
                        source: index.resolve_special(&target.name, "Receive"),
                        visibility: Visibility::External,
                        mutability: StateMutability::Payable,
                        modifiers: Vec::new(),
                    });
                }
                Item::Fallback(fallback) => {
                    special.push(ExternalFunctionInfo {
                        name: "fallback".to_owned(),
                        signature: "fallback()".to_owned(),
                        selector: None,
                        source: index.resolve_special(&target.name, "Fallback"),
                        visibility: Visibility::External,
                        mutability: fallback.state_mutability.clone(), // checkrs: allow(clone_in_loops)
                        modifiers: Vec::new(),
                    });
                }
                _ => {}
            }
        }

        // 7. Sort each section so repeated runs render identically.
        for section in [&mut mutable, &mut view, &mut callback, &mut special] {
            section.sort_by(|a, b| a.signature.cmp(&b.signature));
        }

        Ok(ExternalFunctionsOutput {
            contract_name: target.name.clone(),
            source_file: compiled.source_path.display().to_string(),
            mutable,
            view,
            callback,
            special,
        })
    }
}

/// Canonical ABI signature used to look up the compiled selector, e.g.
/// `allocate(uint256,(uint256,uint128)[])`.
fn canonical_signature(function: &AbiFunction) -> String {
    let params = function
        .inputs
        .iter()
        .map(|param| canonical_type(&param.r#type, &param.components))
        .collect::<Vec<String>>()
        .join(",");
    format!("{}({})", function.name, params)
}

/// Canonical ABI type of a parameter or component with tuple components
/// expanded, so `tuple[]` with two members renders as `(uint256,address)[]`.
fn canonical_type(r#type: &str, components: &Option<Vec<Component>>) -> String {
    let Some(components) = components else {
        return r#type.to_owned();
    };
    let inner = components
        .iter()
        .map(|component| canonical_type(&component.r#type, &component.components))
        .collect::<Vec<String>>()
        .join(",");
    let suffix = r#type.strip_prefix("tuple").unwrap_or("");
    format!("({inner}){suffix}")
}

/// Readable signature of an ABI function.
fn display_signature(function: &AbiFunction) -> String {
    let params = function
        .inputs
        .iter()
        .map(display_param_type)
        .collect::<Vec<String>>()
        .join(",");
    format!("{}({})", function.name, params)
}

/// Readable type of an ABI parameter.
///
/// Struct and enum internal types lose the `struct`/`enum` marker and the
/// defining contract qualifier, so `struct Voter.Allocation[]` renders as
/// `Allocation[]`.
fn display_param_type(param: &Param) -> String {
    if let Some(internal) = &param.internal_type
        && let Some(rest) = internal
            .strip_prefix("struct ")
            .or_else(|| internal.strip_prefix("enum "))
    {
        return rest.rsplit('.').next().unwrap_or(rest).to_owned();
    }
    param.r#type.clone()
}

/// Canonical parameter key used to match an ABI function against the AST.
fn abi_param_key(function: &AbiFunction) -> String {
    function
        .inputs
        .iter()
        .map(|param| normalize_type(param.internal_type.as_deref().unwrap_or(&param.r#type)))
        .collect::<Vec<String>>()
        .join(",")
}

/// Parameter key of an AST function declaration.
fn ast_param_key(params: &[VariableDeclaration]) -> String {
    params
        .iter()
        .map(|param| normalize_type(param.type_descriptions.type_string.as_deref().unwrap_or("")))
        .collect::<Vec<String>>()
        .join(",")
}

/// Normalizes solc and ABI type strings so both sides use the same key.
fn normalize_type(kind: &str) -> String {
    kind.replace(" memory", "")
        .replace(" calldata", "")
        .replace(" storage", "")
        .replace(" payable", "")
}

/// Returns `true` for well-known token receiver callbacks.
fn is_callback(name: &str) -> bool {
    matches!(
        name,
        "onERC721Received" | "onERC1155Received" | "onERC1155BatchReceived"
    )
}

/// Lowercase label for an ABI state mutability.
fn mutability_label(mutability: &StateMutability) -> &'static str {
    match mutability {
        StateMutability::Pure => "pure",
        StateMutability::View => "view",
        StateMutability::Nonpayable => "nonpayable",
        StateMutability::Payable => "payable",
    }
}

/// 1-based line number of a byte offset within a compiled source.
///
/// Returns `0` when the source content is unavailable.
fn line_of(sources: &HashMap<PathBuf, String>, path: impl AsRef<Path>, offset: usize) -> usize {
    let Some(content) = sources.get(path.as_ref()) else {
        return 0;
    };
    let bytes = content.as_bytes();
    let offset = offset.min(bytes.len());
    bytes[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

/// A resolved public or external declaration from the compiled ASTs.
#[derive(Clone)]
struct FuncInfo {
    file: String,
    line: usize,
    visibility: Visibility,
    modifiers: Vec<String>,
    /// Normalized parameter types, empty for public state variable getters.
    signature: String,
}

/// Index of public and external declarations across the compiled sources.
struct FunctionIndex {
    /// Key: `(contract_name, function_name)` with one entry per overload.
    functions: HashMap<(String, String), Vec<FuncInfo>>,

    /// Key: `(contract_name, kind)` for receive and fallback definitions.
    specials: HashMap<(String, String), FuncInfo>,

    /// Direct base contract names of every indexed contract.
    bases: HashMap<String, Vec<String>>,
}

impl FunctionIndex {
    /// Builds the index from every AST in the compilation output.
    fn build(output: &StandardJSONOutput, sources: &HashMap<PathBuf, String>) -> Self {
        let mut index = Self {
            functions: HashMap::new(),
            specials: HashMap::new(),
            bases: HashMap::new(),
        };
        for (path, source) in &output.sources {
            let Some(ast) = &source.ast else { continue };
            for node in &ast.nodes {
                if let SourceUnitNode::ContractDefinition(contract) = node {
                    index.contract(contract, path, sources);
                }
            }
        }
        index
    }

    /// Indexes the externally callable declarations of one contract.
    fn contract(
        &mut self,
        contract: &ContractDefinition,
        path: impl AsRef<Path>,
        sources: &HashMap<PathBuf, String>,
    ) {
        let path = path.as_ref();
        // 1. Record the direct base contracts for inheritance walks.
        if !contract.base_contracts.is_empty() {
            // checkrs: allow(clone_in_iterator)
            let bases: Vec<String> = contract
                .base_contracts
                .iter()
                // checkrs: allow(clone_in_iterator)
                .map(|base| base.base_name.name.clone())
                .collect();
            self.bases
                .entry(contract.name.clone())
                .or_default()
                .extend(bases);
        }

        // 2. Index the externally callable functions and public getters.
        for node in &contract.nodes {
            match node {
                ContractDefinitionNode::FunctionDefinition(function)
                    if function.visibility == Visibility::External
                        || function.visibility == Visibility::Public =>
                {
                    self.function(&contract.name, function, path, sources);
                }
                ContractDefinitionNode::VariableDeclaration(variable)
                    if variable.visibility == Visibility::Public =>
                {
                    let info = FuncInfo {
                        file: path.display().to_string(),
                        line: line_of(sources, path, variable.src.offset),
                        visibility:
                            // checkrs: allow(clone_in_loops)
                            variable.visibility.clone(),
                        modifiers: Vec::new(),
                        signature: String::new(),
                    };
                    self.functions
                        // checkrs: allow(clone_in_loops)
                        .entry((contract.name.clone(), variable.name.clone()))
                        .or_default()
                        .push(info);
                }
                _ => {}
            }
        }
    }

    /// Indexes one function definition or special function.
    fn function(
        &mut self,
        contract_name: &str,
        function: &FunctionDefinition,
        path: impl AsRef<Path>,
        sources: &HashMap<PathBuf, String>,
    ) {
        let path = path.as_ref();
        let info = FuncInfo {
            file: path.display().to_string(),
            line: line_of(sources, path, function.src.offset),
            visibility: function.visibility.clone(),
            // checkrs: allow(clone_in_iterator)
            modifiers: function
                .modifiers
                .iter()
                // checkrs: allow(clone_in_iterator)
                .map(|modifier| modifier.modifier_name.name.clone())
                .collect(),
            signature: ast_param_key(&function.parameters.parameters),
        };
        match Self::kind(function) {
            FunctionKind::Receive => {
                self.specials
                    .insert((contract_name.to_owned(), "Receive".to_owned()), info);
            }
            FunctionKind::Fallback => {
                self.specials
                    .insert((contract_name.to_owned(), "Fallback".to_owned()), info);
            }
            FunctionKind::Function => {
                self.functions
                    .entry((contract_name.to_owned(), function.name.clone()))
                    .or_default()
                    .push(info);
            }
            FunctionKind::Constructor | FunctionKind::FreeFunction => {}
        }
    }

    /// Resolves the Solidity function kind, inferring constructor and
    /// fallback from older ASTs where `kind` is absent.
    fn kind(function: &FunctionDefinition) -> FunctionKind {
        function.kind.clone().unwrap_or({
            if function.is_constructor {
                FunctionKind::Constructor
            } else if function.name.is_empty() {
                FunctionKind::Fallback
            } else {
                FunctionKind::Function
            }
        })
    }

    /// Resolves the declaration matching an ABI function.
    ///
    /// Walks the inheritance chain first, matching exact parameter
    /// signatures, then public state variable getters, and finally any
    /// declaration of the same name in the compilation unit.
    fn resolve(&self, contract_name: &str, name: &str, abi_key: &str) -> Option<FuncInfo> {
        let ancestors = self.ancestors(contract_name);

        // 1. Exact signature match up the inheritance chain.
        if let Some(info) = self.find(&ancestors, name, |info| info.signature == abi_key) {
            return Some(info.clone());
        }

        // 2. Public state variable getters have an empty AST signature.
        if let Some(info) = self.find(&ancestors, name, |info| info.signature.is_empty()) {
            return Some(info.clone());
        }

        // 3. Fall back to any declaration of the same name in the unit.
        let mut candidates: Vec<&FuncInfo> = self
            .functions
            .iter()
            .filter(|((_, key_name), _)| key_name == name)
            .flat_map(|(_, infos)| infos)
            .collect();
        if let Some(info) = candidates.iter().find(|info| info.signature == abi_key) {
            return Some((*info).clone());
        }
        candidates.sort_by_key(|info| (info.line, info.file.clone()));
        candidates.into_iter().next().cloned()
    }

    /// Finds the first declaration matching `matches` up the inheritance
    /// chain.
    fn find(
        &self,
        ancestors: &[String],
        name: &str,
        matches: impl Fn(&FuncInfo) -> bool,
    ) -> Option<&FuncInfo> {
        for ancestor in ancestors {
            // checkrs: allow(clone_in_loops)
            let key = (ancestor.clone(), name.to_owned());
            if let Some(info) = self
                .functions
                .get(&key)
                .and_then(|infos| infos.iter().find(|info| matches(info)))
            {
                return Some(info);
            }
        }
        None
    }

    /// Names of the contract followed by its transitive base contracts.
    fn ancestors(&self, contract_name: &str) -> Vec<String> {
        // 1. Walk the direct base contracts breadth first.
        let mut ancestors = vec![contract_name.to_owned()];
        let mut seen: HashSet<String> = HashSet::from([contract_name.to_owned()]);
        let mut head = 0;
        while head < ancestors.len() {
            // checkrs: allow(clone_in_loops)
            let name = ancestors[head].clone();
            head += 1;
            // checkrs: allow(nested_if_let)
            if let Some(bases) = self.bases.get(&name) {
                for base in bases {
                    // checkrs: allow(clone_in_loops)
                    if seen.insert(base.clone()) {
                        // checkrs: allow(clone_in_loops)
                        ancestors.push(base.clone());
                    }
                }
            }
        }
        ancestors
    }

    /// Resolves the receive or fallback declaration nearest in the
    /// inheritance chain.
    fn resolve_special(&self, contract_name: &str, kind: &str) -> Option<SourceInfo> {
        for ancestor in self.ancestors(contract_name) {
            // checkrs: allow(clone_in_loops)
            if let Some(info) = self.specials.get(&(ancestor, kind.to_owned())) {
                return Some(SourceInfo {
                    // checkrs: allow(clone_in_loops)
                    file: info.file.clone(),
                    line: info.line,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(r#type: &str, internal: Option<&str>) -> Param {
        Param {
            name: String::new(),
            r#type: r#type.to_owned(),
            components: None,
            internal_type: internal.map(|kind| kind.to_owned()),
        }
    }

    #[test]
    fn display_param_type_strips_struct_and_enum_markers() {
        assert_eq!(
            display_param_type(&param("tuple", Some("struct Voter.Allocation[]"))),
            "Allocation[]"
        );
        assert_eq!(
            display_param_type(&param("tuple", Some("enum Base.Kind"))),
            "Kind"
        );
    }

    #[test]
    fn display_param_type_falls_back_to_the_abi_type() {
        assert_eq!(display_param_type(&param("uint256", None)), "uint256");
        assert_eq!(
            display_param_type(&param("address", Some("contract IERC20"))),
            "address"
        );
    }
}
