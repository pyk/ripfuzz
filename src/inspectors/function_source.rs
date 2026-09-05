//! Function source inspection for ripfuzz projects.
//!
//! [`FunctionSourceInspector`] resolves the complete source code of a
//! function selected by its 4-byte selector, together with every symbol the
//! function references: internal functions, modifiers, structs, enums,
//! errors, events, state variables, and inherited declarations across the
//! whole compilation unit.
//!
//! ```rust
//! use ripfuzz::config::Config;
//! use ripfuzz::harness::HarnessId;
//! use ripfuzz::inspectors::FunctionSourceInspector;
//!
//! let root = std::path::Path::new(".");
//! let config = Config::new().with_root(root).load("ripfuzz.toml")?;
//! let target = HarnessId::try_from("src/Voter.sol:Voter")?;
//! // let report = FunctionSourceInspector::new(root, config).inspect(&target, "f02e634e")?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use solc::StandardJSONOutput;
use solc::ast::{
    ContractDefinition, ContractDefinitionNode, ContractKind, Expression, FunctionCallExpression,
    FunctionDefinition, FunctionKind, ModifierInvocation, ModifierInvocationKind, SourceLocation,
    SourceUnit, SourceUnitNode, Statement, TypeName, VariableDeclaration, Visibility,
};

use crate::config::Config;
use crate::harness::HarnessId;
use crate::inspectors::CompiledTarget;

/// Kind of a resolved declaration, driving the section headings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// A function declaration, including constructors and free functions.
    Function,

    /// A modifier definition.
    Modifier,

    /// A state variable declaration.
    Variable,

    /// A struct definition.
    Struct,

    /// An enum definition.
    Enum,

    /// A custom error definition.
    Error,

    /// An event definition.
    Event,

    /// A user-defined value type definition.
    UserDefinedValueType,

    /// A contract, abstract contract, interface, or library definition.
    Contract {
        /// The Solidity contract kind.
        kind: ContractKind,

        /// Whether the contract is marked `abstract`.
        is_abstract: bool,
    },
}

impl SymbolKind {
    /// Section heading of the declaration kind.
    pub fn heading(&self) -> &'static str {
        match self {
            Self::Function => "Function",
            Self::Modifier => "Modifier",
            Self::Variable => "Variable",
            Self::Struct => "Struct",
            Self::Enum => "Enum",
            Self::Error => "Error",
            Self::Event => "Event",
            Self::UserDefinedValueType => "User Defined Value Type",
            Self::Contract { kind, is_abstract } => match kind {
                ContractKind::Contract if *is_abstract => "Abstract Contract",
                ContractKind::Contract => "Contract",
                ContractKind::Interface => "Interface",
                ContractKind::Library => "Library",
            },
        }
    }

    /// Whether the declaration is a contract, interface, or library.
    fn is_contract(&self) -> bool {
        matches!(self, Self::Contract { .. })
    }
}

/// A resolved declaration with its source range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSymbol {
    /// Symbol string, e.g. `Voter.allocate(uint256,...)` for a function,
    /// `Voter.owner` for a getter, or a plain name for other declarations.
    pub symbol: String,

    /// File holding the declaration, relative to the project root.
    pub file: PathBuf,

    /// Byte offset of the declaration within the file.
    pub offset: usize,

    /// Byte length of the declaration.
    pub length: usize,

    /// Kind of the declaration.
    pub kind: SymbolKind,
}

/// A resolved symbol together with its rendered natspec.
#[derive(Debug)]
struct Rendered {
    resolved: ResolvedSymbol,

    natspec: String,
}

/// The report produced by [`FunctionSourceInspector`].
#[derive(Debug)]
pub struct FunctionSourceOutput {
    /// Resolved symbols, the inspected function first.
    symbols: Vec<Rendered>,

    /// Contents of every compiled source, keyed relative to the project root
    /// and LF-normalized, so slicing matches the solc `src` offsets.
    sources: HashMap<PathBuf, String>,
}

impl FunctionSourceOutput {
    /// Resolved symbols in report order, the inspected function first.
    pub fn symbols(&self) -> Vec<ResolvedSymbol> {
        self.symbols
            .iter()
            // checkrs: allow(clone_in_iterator)
            .map(|rendered| rendered.resolved.clone())
            .collect()
    }
}

impl std::fmt::Display for FunctionSourceOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 1. Root function section.
        let Some(root) = self.symbols.first() else {
            return Ok(());
        };
        writeln!(
            f,
            "# {} - {} Source Code",
            contract_of(&root.resolved.symbol),
            display_name(&root.resolved.symbol, &root.resolved.kind)
        )?;
        writeln!(f)?;
        self.write_section(f, root, true)?;

        // 2. Referenced symbol sections, sorted by heading and name. The
        //    file and offset keep the order deterministic across same-named
        //    declarations.
        let mut rest: Vec<&Rendered> = self.symbols.iter().skip(1).collect();
        rest.sort_by(|a, b| {
            let a_key = (
                a.resolved.kind.heading(),
                display_name(&a.resolved.symbol, &a.resolved.kind).to_lowercase(),
                a.resolved.file.clone(),
                a.resolved.offset,
            );
            let b_key = (
                b.resolved.kind.heading(),
                display_name(&b.resolved.symbol, &b.resolved.kind).to_lowercase(),
                b.resolved.file.clone(),
                b.resolved.offset,
            );
            a_key.cmp(&b_key)
        });
        for rendered in rest {
            writeln!(f)?;
            writeln!(f, "---")?;
            writeln!(f)?;
            self.write_section(f, rendered, false)?;
        }
        Ok(())
    }
}

impl FunctionSourceOutput {
    /// Writes one symbol as a markdown section.
    ///
    /// The root section carries the report title instead of a heading, so it
    /// skips the `## Kind: name` line.
    fn write_section(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        rendered: &Rendered,
        root: bool,
    ) -> std::fmt::Result {
        let symbol = &rendered.resolved;
        if !root {
            writeln!(
                f,
                "## {}: `{}`",
                symbol.kind.heading(),
                display_name(&symbol.symbol, &symbol.kind)
            )?;
            writeln!(f)?;
        }

        // 1. Fallback block when the file content is unavailable.
        let Some(content) = self.sources.get(&symbol.file) else {
            writeln!(f, "Source path: `{}`", symbol.file.display())?;
            writeln!(f)?;
            writeln!(f, "```solidity")?;
            writeln!(f, "// unable to read")?;
            writeln!(f, "```")?;
            return Ok(());
        };

        // 2. Dedented declaration source. Contract symbols render their
        //    header up to the opening brace only, because their range spans
        //    every member body.
        let line = line_of(content, symbol.offset);
        let base = base_indent(content, symbol.offset);
        let text = if symbol.kind.is_contract() {
            contract_header(content, symbol.offset).to_owned()
        } else {
            slice(content, symbol.offset, symbol.length)
        };
        let text = dedent(&text, base);

        // 3. Code block with the natspec preceding the declaration.
        writeln!(f, "Source path: `{}:{line}`", symbol.file.display())?;
        writeln!(f)?;
        writeln!(f, "```solidity")?;
        if !rendered.natspec.is_empty() {
            writeln!(f, "{}", rendered.natspec.trim_end())?;
        }
        writeln!(f, "{}", text.trim_end())?;
        writeln!(f, "```")
    }
}

/// Inspects the complete source code of a Solidity function selector.
///
/// The inspector compiles the target through the shared solc pipeline, so a
/// cached compilation keyed by the standard JSON input hash skips solc
/// entirely on repeated runs.
pub struct FunctionSourceInspector {
    root: PathBuf,
    config: Config,
}

impl FunctionSourceInspector {
    /// Creates an inspector for a project root and its loaded config.
    pub fn new(root: impl AsRef<Path>, config: Config) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            config,
        }
    }

    /// Inspects the function of `target` selected by `selector`.
    pub fn inspect(&self, target: &HarnessId, selector: &str) -> Result<FunctionSourceOutput> {
        // 1. Compile the target or reuse a cached compilation.
        let compiled = CompiledTarget::compile(&self.root, &self.config, target)?;

        // 2. Index every declaration across the compiled ASTs.
        let index = SymbolIndex::build(&compiled.output);

        // 3. Resolve the selector to the root declaration.
        let (root, root_id) = resolve_root(target, selector, &compiled, &index)?;

        // 4. Collect the declarations the root references.
        let symbols = resolve_recursive(root, root_id, &compiled.output, &index);

        // 5. Render the natspec of every resolved symbol.
        let mut rendered = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            let content = compiled.sources.get(&symbol.file);
            let raw = content
                .map(|source| extract_natspec(source, symbol.offset))
                .unwrap_or_default();
            let natspec = resolve_inheritdoc(&index, &compiled.sources, &symbol.symbol, &raw);
            let natspec = match content {
                Some(source) => dedent(&natspec, base_indent(source, symbol.offset)),
                None => natspec,
            };
            rendered.push(Rendered {
                resolved: symbol,
                natspec,
            });
        }

        // 6. LF-normalize the source contents, so slicing matches the solc
        //    `src` offsets.
        let sources = compiled
            .sources
            .iter()
            // checkrs: allow(clone_in_iterator)
            .map(|(file, content)| (file.clone(), content.replace('\r', "")))
            .collect();

        Ok(FunctionSourceOutput {
            symbols: rendered,
            sources,
        })
    }
}

/// Resolves the declaration of `target` selected by `selector`.
///
/// Returns the resolved symbol together with its index node id.
fn resolve_root(
    target: &HarnessId,
    selector: &str,
    compiled: &CompiledTarget,
    index: &SymbolIndex,
) -> Result<(ResolvedSymbol, i64)> {
    // 1. Normalize the selector, accepting an optional `0x` prefix.
    let trimmed = selector.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed)
        .to_lowercase();
    ensure!(
        hex.len() == 8 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "selector must be 8 hex characters, got `{selector}`"
    );

    // 2. Find the target contract in the compiled source file. The ABI
    //    lookup doubles as an existence check with a precise error.
    compiled.contract(target)?;
    let contract_id = index
        .contracts
        .get(&target.name)
        .and_then(|candidates| {
            candidates
                .iter()
                .find(|(_, file)| file == &compiled.source_path)
                .map(|(id, _)| *id)
        })
        .with_context(|| {
            format!(
                "contract `{}` not found in the compiled AST of `{}`",
                target.name,
                compiled.source_path.display()
            )
        })?;

    // 3. Walk the inheritance chain, most-derived first, for a declaration
    //    carrying the selector.
    for ancestor in index.ancestors(contract_id) {
        let Some(members) = index.members.get(&ancestor) else {
            continue;
        };
        for member_id in members {
            let Some(entry) = index.entries.get(member_id) else {
                continue;
            };
            if entry.selector.as_deref() == Some(hex.as_str())
                && let Some(symbol) = index.symbol(*member_id)
            {
                return Ok((symbol, *member_id));
            }
        }
    }

    // 4. List the available selectors of the target contract.
    let mut available: Vec<(&String, &String)> = compiled
        .contract(target)?
        .evm
        .as_ref()
        .and_then(|evm| evm.method_identifiers.as_ref())
        .map(|identifiers| identifiers.iter().collect())
        .unwrap_or_default();
    available.sort();
    let list = available
        .iter()
        .map(|(signature, selector)| format!("  {selector}  {signature}"))
        .collect::<Vec<String>>()
        .join("\n");
    bail!(
        "selector `{hex}` not found in contract `{}`, available selectors:\n{list}",
        target.name
    )
}

/// Collects the root declaration and every transitively referenced one.
///
/// Contract symbols contribute their header only, because their range spans
/// every member body, so walking them would pull in the whole contract.
fn resolve_recursive(
    root: ResolvedSymbol,
    root_id: i64,
    output: &StandardJSONOutput,
    index: &SymbolIndex,
) -> Vec<ResolvedSymbol> {
    // 1. Most-derived override table of the inspected chain.
    let overrides = index
        .entries
        .get(&root_id)
        .and_then(|entry| entry.contract_id)
        .map(|contract_id| OverrideTable::build(index, contract_id))
        .unwrap_or_default();

    // 2. Walk the reference graph, deduplicating by node id.
    let mut resolved: Vec<ResolvedSymbol> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue: Vec<(i64, ResolvedSymbol)> = vec![(root_id, root)];
    while let Some((id, symbol)) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        if !symbol.kind.is_contract()
            && let Some(ast) = ast_of(output, &symbol.file)
        {
            let references = collect_references(ast, &symbol, id, index, &overrides);
            for reference in references {
                if let Some(referenced) = index.symbol(reference) {
                    queue.push((reference, referenced));
                }
            }
        }
        resolved.push(symbol);
    }
    resolved
}

/// Most-derived function overrides of the inspected contract chain.
///
/// Unqualified calls such as `_afterTokenTransfer(...)` inside a parent
/// function point at the parent declaration. This table redirects such calls
/// to the most-derived override of the inspected contract. Explicitly
/// qualified calls, `super.fn` or `Base.fn`, stay untouched.
#[derive(Default)]
struct OverrideTable {
    /// Contract ids of the inspected inheritance chain.
    chain: HashSet<i64>,

    /// Most-derived implemented function per `(name, params)`.
    most_derived: HashMap<(String, String), i64>,

    /// Virtual or override functions keyed by `(contract, name, params)`.
    dispatchable: HashSet<(i64, String, String)>,
}

impl OverrideTable {
    /// Collects the overridable functions across the inheritance chain.
    fn build(index: &SymbolIndex, contract_id: i64) -> Self {
        let mut table = Self::default();
        for ancestor in index.ancestors(contract_id) {
            table.chain.insert(ancestor);
            let Some(members) = index.members.get(&ancestor) else {
                continue;
            };
            for member_id in members {
                let Some(entry) = index.entries.get(member_id) else {
                    continue;
                };
                if entry.kind != SymbolKind::Function
                    || entry.function_kind != FunctionKind::Function
                    || entry.visibility == Visibility::Private
                {
                    continue;
                }
                // solc < 0.6 omits `virtual`, so functions are implicitly
                // dispatchable and resolve to the most-derived override.
                if entry.overridable {
                    table
                        .dispatchable
                        // checkrs: allow(clone_in_loops)
                        .insert((ancestor, entry.name.clone(), entry.params.clone()));
                }
                if entry.implemented {
                    table
                        .most_derived
                        // checkrs: allow(clone_in_loops)
                        .entry((entry.name.clone(), entry.params.clone()))
                        .or_insert(*member_id);
                }
            }
        }
        table
    }

    /// Redirects an unqualified call to the most-derived override.
    fn redirect(&self, index: &SymbolIndex, referenced: i64) -> Option<i64> {
        let entry = index.entries.get(&referenced)?;
        if entry.kind != SymbolKind::Function {
            return None;
        }
        let contract_id = entry.contract_id?;
        let on_chain = self.chain.contains(&contract_id);
        if !on_chain {
            return None;
        }
        let name = entry.name.clone();
        let params = entry.params.clone();
        let dispatchable = self
            .dispatchable
            .contains(&(contract_id, name.clone(), params.clone()));
        if !dispatchable {
            return None;
        }
        let key = (name, params);
        let best = *self.most_derived.get(&key)?;
        if best == referenced {
            return None;
        }
        Some(best)
    }
}

/// A declaration indexed by its Solc node id.
struct IndexEntry {
    id: i64,

    /// File holding the declaration, keyed like the compiled sources.
    file: PathBuf,

    /// Byte offset of the declaration within the file.
    offset: usize,

    /// Byte length of the declaration.
    length: usize,

    /// Declaration name, e.g. `allocate`.
    name: String,

    /// Owning contract id, `None` for top-level declarations.
    contract_id: Option<i64>,

    /// Owning contract name, empty at top level.
    contract: String,

    /// Kind of the declaration.
    kind: SymbolKind,

    /// Normalized parameter type key of functions and modifiers.
    params: String,

    /// Readable parameter types of functions, e.g. `uint256,Point`.
    formatted_params: Option<String>,

    /// Solidity function kind, `FunctionKind::Function` otherwise.
    function_kind: FunctionKind,

    /// Visibility of the declaration.
    visibility: Visibility,

    /// Whether the declaration carries a body.
    implemented: bool,

    /// Whether a function can be overridden.
    overridable: bool,

    /// 4-byte selector of external and public functions and public getters.
    selector: Option<String>,
}

/// Index of every declaration across the compiled ASTs, keyed by Solc node
/// id. A single compilation owns one global id namespace, so the ids need no
/// scoping.
struct SymbolIndex {
    entries: HashMap<i64, IndexEntry>,

    /// Direct base contract ids of every contract.
    bases: HashMap<i64, Vec<i64>>,

    /// Contract ids by name, with their source file.
    contracts: HashMap<String, Vec<(i64, PathBuf)>>,

    /// Member entry ids of every contract, keyed by contract id.
    members: HashMap<i64, Vec<i64>>,

    /// Implemented constructor entry id of every contract.
    constructors: HashMap<i64, i64>,
}

impl SymbolIndex {
    /// Builds the index from every AST in the compilation output.
    fn build(output: &StandardJSONOutput) -> Self {
        let mut index = Self {
            entries: HashMap::new(),
            bases: HashMap::new(),
            contracts: HashMap::new(),
            members: HashMap::new(),
            constructors: HashMap::new(),
        };
        for (file, source) in &output.sources {
            let Some(ast) = &source.ast else { continue };
            for node in &ast.nodes {
                match node {
                    SourceUnitNode::ContractDefinition(contract) => {
                        index.contract(contract, file);
                    }
                    SourceUnitNode::FunctionDefinition(function) => {
                        index.function(function, file, None);
                    }
                    SourceUnitNode::VariableDeclaration(variable) => {
                        index.variable(variable, file, None);
                    }
                    SourceUnitNode::StructDefinition(definition) => {
                        let mut entry = Self::declaration(
                            definition.id,
                            &definition.name,
                            &definition.src,
                            file,
                            None,
                        );
                        entry.kind = SymbolKind::Struct;
                        index.push(entry);
                    }
                    SourceUnitNode::EnumDefinition(definition) => {
                        let mut entry = Self::declaration(
                            definition.id,
                            &definition.name,
                            &definition.src,
                            file,
                            None,
                        );
                        entry.kind = SymbolKind::Enum;
                        index.push(entry);
                    }
                    SourceUnitNode::ErrorDefinition(definition) => {
                        let mut entry = Self::declaration(
                            definition.id,
                            &definition.name,
                            &definition.src,
                            file,
                            None,
                        );
                        entry.kind = SymbolKind::Error;
                        index.push(entry);
                    }
                    SourceUnitNode::EventDefinition(definition) => {
                        let mut entry = Self::declaration(
                            definition.id,
                            &definition.name,
                            &definition.src,
                            file,
                            None,
                        );
                        entry.kind = SymbolKind::Event;
                        index.push(entry);
                    }
                    SourceUnitNode::UserDefinedValueTypeDefinition(definition) => {
                        let mut entry = Self::declaration(
                            definition.id,
                            &definition.name,
                            &definition.src,
                            file,
                            None,
                        );
                        entry.kind = SymbolKind::UserDefinedValueType;
                        index.push(entry);
                    }
                    _ => {}
                }
            }
        }
        index
    }

    /// Shared fields of every indexed declaration.
    fn declaration(
        id: i64,
        name: &str,
        src: &SourceLocation,
        file: &Path,
        owner: Option<(i64, &str)>,
    ) -> IndexEntry {
        IndexEntry {
            id,
            file: file.to_path_buf(),
            offset: src.offset,
            length: src.length,
            name: name.to_owned(),
            contract_id: owner.map(|(contract_id, _)| contract_id),
            contract: owner.map(|(_, name)| name.to_owned()).unwrap_or_default(),
            kind: SymbolKind::Function,
            params: String::new(),
            formatted_params: None,
            function_kind: FunctionKind::Function,
            visibility: Visibility::Public,
            implemented: false,
            overridable: false,
            selector: None,
        }
    }

    /// Inserts one entry.
    fn push(&mut self, entry: IndexEntry) {
        self.entries.insert(entry.id, entry);
    }

    /// Records a contract member, keeping the member list ordered.
    fn track_member(&mut self, contract_id: i64, member_id: i64) {
        self.members.entry(contract_id).or_default().push(member_id);
    }

    /// Indexes a contract declaration and its members.
    fn contract(&mut self, contract: &ContractDefinition, file: &Path) {
        // 1. Index the contract declaration itself.
        let mut entry = Self::declaration(contract.id, "", &contract.src, file, None);
        entry.name = contract.name.clone();
        entry.contract = contract.name.clone();
        entry.contract_id = Some(contract.id);
        entry.kind = SymbolKind::Contract {
            kind: contract.contract_kind.clone(),
            is_abstract: contract.r#abstract.unwrap_or(false),
        };
        entry.implemented = contract.fully_implemented;
        self.push(entry);
        self.contracts
            .entry(contract.name.clone())
            .or_default()
            .push((contract.id, file.to_path_buf()));

        // 2. Record the direct base contract ids for inheritance walks.
        for base in &contract.base_contracts {
            if let Some(id) = base.base_name.referenced_declaration {
                self.bases.entry(contract.id).or_default().push(id);
            }
        }

        // 3. Index the members.
        let owner = Some((contract.id, contract.name.as_str()));
        for node in &contract.nodes {
            match node {
                ContractDefinitionNode::FunctionDefinition(function) => {
                    self.function(function, file, owner);
                }
                ContractDefinitionNode::ModifierDefinition(modifier) => {
                    let mut entry =
                        Self::declaration(modifier.id, &modifier.name, &modifier.src, file, owner);
                    entry.kind = SymbolKind::Modifier;
                    entry.params = ast_param_key(&modifier.parameters.parameters);
                    entry.implemented = true;
                    self.track_member(contract.id, modifier.id);
                    self.push(entry);
                }
                ContractDefinitionNode::VariableDeclaration(variable) => {
                    self.variable(variable, file, owner);
                    self.track_member(contract.id, variable.id);
                }
                ContractDefinitionNode::StructDefinition(definition) => {
                    let mut entry = Self::declaration(
                        definition.id,
                        &definition.name,
                        &definition.src,
                        file,
                        owner,
                    );
                    entry.kind = SymbolKind::Struct;
                    self.track_member(contract.id, definition.id);
                    self.push(entry);
                }
                ContractDefinitionNode::EnumDefinition(definition) => {
                    let mut entry = Self::declaration(
                        definition.id,
                        &definition.name,
                        &definition.src,
                        file,
                        owner,
                    );
                    entry.kind = SymbolKind::Enum;
                    self.track_member(contract.id, definition.id);
                    self.push(entry);
                }
                ContractDefinitionNode::ErrorDefinition(definition) => {
                    let mut entry = Self::declaration(
                        definition.id,
                        &definition.name,
                        &definition.src,
                        file,
                        owner,
                    );
                    entry.kind = SymbolKind::Error;
                    self.track_member(contract.id, definition.id);
                    self.push(entry);
                }
                ContractDefinitionNode::EventDefinition(definition) => {
                    let mut entry = Self::declaration(
                        definition.id,
                        &definition.name,
                        &definition.src,
                        file,
                        owner,
                    );
                    entry.kind = SymbolKind::Event;
                    self.track_member(contract.id, definition.id);
                    self.push(entry);
                }
                ContractDefinitionNode::UserDefinedValueTypeDefinition(definition) => {
                    let mut entry = Self::declaration(
                        definition.id,
                        &definition.name,
                        &definition.src,
                        file,
                        owner,
                    );
                    entry.kind = SymbolKind::UserDefinedValueType;
                    self.track_member(contract.id, definition.id);
                    self.push(entry);
                }
                ContractDefinitionNode::UsingForDirective(_) => {}
            }
        }
    }

    /// Indexes a function definition.
    fn function(&mut self, function: &FunctionDefinition, file: &Path, owner: Option<(i64, &str)>) {
        let kind = function_kind(function);
        let mut entry = Self::declaration(function.id, &function.name, &function.src, file, owner);
        entry.kind = SymbolKind::Function;
        entry.params = ast_param_key(&function.parameters.parameters);
        entry.formatted_params = Some(format_params(&function.parameters.parameters));
        entry.function_kind = kind.clone();
        entry.visibility = function.visibility.clone();
        entry.implemented = function.implemented;
        entry.overridable = function.r#virtual.unwrap_or(true) || function.overrides.is_some();
        entry.selector = function
            .function_selector
            .as_deref()
            .map(|selector| selector.to_lowercase());
        let constructor_id = owner
            .map(|(contract_id, _)| contract_id)
            .filter(|_| kind == FunctionKind::Constructor && function.implemented);
        if let Some((contract_id, _)) = owner {
            self.track_member(contract_id, function.id);
        }
        if let Some(constructor_id) = constructor_id {
            self.constructors.insert(constructor_id, function.id);
        }
        self.push(entry);
    }

    /// Indexes a state variable declaration.
    fn variable(
        &mut self,
        variable: &VariableDeclaration,
        file: &Path,
        owner: Option<(i64, &str)>,
    ) {
        let mut entry = Self::declaration(variable.id, &variable.name, &variable.src, file, owner);
        entry.kind = SymbolKind::Variable;
        entry.visibility = variable.visibility.clone();
        entry.implemented = true;
        entry.selector = variable
            .function_selector
            .as_deref()
            .map(|selector| selector.to_lowercase());
        self.push(entry);
    }

    /// Names of the contract followed by its transitive base contracts.
    fn ancestors(&self, contract_id: i64) -> Vec<i64> {
        // 1. Walk the direct base contracts breadth first.
        let mut ancestors = vec![contract_id];
        let mut seen: HashSet<i64> = HashSet::from([contract_id]);
        let mut head = 0;
        while head < ancestors.len() {
            let current = ancestors[head];
            head += 1;
            let bases = self.bases.get(&current);
            for base in bases.into_iter().flatten() {
                if seen.insert(*base) {
                    ancestors.push(*base);
                }
            }
        }
        ancestors
    }

    /// Resolved symbol of an indexed entry.
    ///
    /// Functions render as `Contract.name(readable,types)`, getters as
    /// `Contract.name`, and other declarations as their plain name.
    fn symbol(&self, id: i64) -> Option<ResolvedSymbol> {
        let entry = self.entries.get(&id)?;
        let symbol = match &entry.kind {
            SymbolKind::Function => {
                let params = entry
                    .formatted_params
                    .clone()
                    .unwrap_or_else(|| entry.params.clone());
                if entry.contract.is_empty() {
                    format!("{}({})", entry.name, params)
                } else {
                    format!("{}.{}({})", entry.contract, entry.name, params)
                }
            }
            SymbolKind::Variable if entry.contract.is_empty() => entry.name.clone(),
            SymbolKind::Variable => format!("{}.{}", entry.contract, entry.name),
            _ => entry.name.clone(),
        };
        Some(ResolvedSymbol {
            symbol,
            file: entry.file.clone(),
            offset: entry.offset,
            length: entry.length,
            kind: entry.kind.clone(),
        })
    }

    /// Function entry of a contract member, by declaration name.
    fn member_function(&self, contract_id: i64, name: &str) -> Option<&IndexEntry> {
        let members = self.members.get(&contract_id)?;
        members
            .iter()
            .filter_map(|member_id| self.entries.get(member_id))
            .find(|entry| entry.kind == SymbolKind::Function && entry.name == name)
    }
}

/// AST of a compiled source file.
fn ast_of<'a>(output: &'a StandardJSONOutput, file: &Path) -> Option<&'a SourceUnit> {
    output
        .sources
        .get(file)
        .and_then(|source| source.ast.as_ref())
}

/// Context of one reference-collection walk over a resolved symbol.
///
/// The walker descends statements and expressions, collecting the Solc node
/// ids of every declaration the symbol references, so the parameter count of
/// each collection step stays small.
struct RefWalk<'a> {
    /// Node id of the walked symbol, never re-added as a reference.
    symbol_id: i64,

    /// Referenced declaration ids in walk order.
    results: Vec<i64>,

    /// Ids already collected within this walk.
    seen_ids: HashSet<i64>,

    index: &'a SymbolIndex,

    overrides: &'a OverrideTable,
}

impl RefWalk<'_> {
    /// Resolves a declaration id, skipping the walked symbol itself and ids
    /// the index does not cover, such as builtins.
    fn add(&mut self, id: i64) {
        if id == self.symbol_id || !self.index.entries.contains_key(&id) {
            return;
        }
        if self.seen_ids.insert(id) {
            self.results.push(id);
        }
    }

    /// Redirects an unqualified call to the most-derived override.
    fn add_dispatchable(&mut self, id: i64) {
        let redirected = self.overrides.redirect(self.index, id);
        match redirected {
            Some(best) => {
                if self.seen_ids.insert(best) {
                    self.results.push(best);
                }
            }
            None => self.add(id),
        }
    }

    /// Collects references from a function definition.
    fn function(&mut self, function: &FunctionDefinition, range_start: usize, range_end: usize) {
        let Some(body) = &function.body else {
            return;
        };
        if !overlaps(&body.src, range_start, range_end) {
            return;
        }
        for parameter in &function.parameters.parameters {
            self.type_name(&parameter.type_name);
        }
        for parameter in &function.return_parameters.parameters {
            self.type_name(&parameter.type_name);
        }
        for modifier in &function.modifiers {
            self.modifier(modifier);
            if let Some(arguments) = &modifier.arguments {
                for argument in arguments {
                    self.expression(argument);
                }
            }
        }
        self.statements(&body.statements);
    }

    /// Collects references from one contract member node.
    fn member(&mut self, node: &ContractDefinitionNode, range_start: usize, range_end: usize) {
        match node {
            ContractDefinitionNode::FunctionDefinition(function) => {
                self.function(function, range_start, range_end);
            }
            ContractDefinitionNode::ModifierDefinition(modifier) => {
                if overlaps(&modifier.body.src, range_start, range_end) {
                    for parameter in &modifier.parameters.parameters {
                        self.type_name(&parameter.type_name);
                    }
                    self.statements(&modifier.body.statements);
                }
            }
            ContractDefinitionNode::VariableDeclaration(variable) => {
                self.variable(variable, range_start, range_end);
            }
            ContractDefinitionNode::StructDefinition(definition) => {
                self.struct_definition(definition, range_start, range_end);
            }
            ContractDefinitionNode::ErrorDefinition(definition) => {
                self.parameters(
                    &definition.parameters.parameters,
                    &definition.src,
                    range_start,
                    range_end,
                );
            }
            ContractDefinitionNode::EventDefinition(definition) => {
                self.parameters(
                    &definition.parameters.parameters,
                    &definition.src,
                    range_start,
                    range_end,
                );
            }
            ContractDefinitionNode::UserDefinedValueTypeDefinition(definition) => {
                self.value_type(definition, range_start, range_end);
            }
            _ => {}
        }
    }

    /// Collects references from a state variable declaration.
    fn variable(&mut self, variable: &VariableDeclaration, range_start: usize, range_end: usize) {
        if !overlaps(&variable.src, range_start, range_end) {
            return;
        }
        self.type_name(&variable.type_name);
        if let Some(value) = &variable.value {
            self.expression(value);
        }
    }

    /// Collects references from a struct definition.
    fn struct_definition(
        &mut self,
        definition: &solc::ast::StructDefinition,
        range_start: usize,
        range_end: usize,
    ) {
        if !overlaps(&definition.src, range_start, range_end) {
            return;
        }
        for member in &definition.members {
            self.type_name(&member.type_name);
        }
    }

    /// Collects references from the parameters of an error or event
    /// definition.
    fn parameters(
        &mut self,
        parameters: &[VariableDeclaration],
        src: &SourceLocation,
        range_start: usize,
        range_end: usize,
    ) {
        if !overlaps(src, range_start, range_end) {
            return;
        }
        for parameter in parameters {
            self.type_name(&parameter.type_name);
        }
    }

    /// Collects references from a user-defined value type definition.
    fn value_type(
        &mut self,
        definition: &solc::ast::UserDefinedValueTypeDefinition,
        range_start: usize,
        range_end: usize,
    ) {
        if !overlaps(&definition.src, range_start, range_end) {
            return;
        }
        self.type_name(&definition.underlying_type);
    }

    /// Collects references from a modifier invocation. A base constructor
    /// specifier resolves to the implemented parent constructor.
    fn modifier(&mut self, modifier: &ModifierInvocation) {
        let Some(id) = modifier.modifier_name.referenced_declaration else {
            return;
        };
        self.add(id);
        if is_base_constructor(modifier, self.index)
            && let Some(constructor) = self.index.constructors.get(&id)
        {
            self.add(*constructor);
        }
    }

    /// Collects references from statements.
    fn statements(&mut self, statements: &[Statement]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    /// Collects references from one statement.
    fn statement(&mut self, statement: &Statement) {
        match statement {
            Statement::ExpressionStatement(statement) => {
                self.expression(&statement.expression);
            }
            Statement::Block(block) => self.statements(&block.statements),
            Statement::IfStatement(statement) => {
                self.expression(&statement.condition);
                self.statement(&statement.true_body);
                if let Some(false_body) = &statement.false_body {
                    self.statement(false_body);
                }
            }
            Statement::ForStatement(statement) => {
                if let Some(initialization) = &statement.initialization_expression {
                    self.expression(initialization);
                }
                self.expression(&statement.condition);
                if let Some(loop_expression) = &statement.loop_expression {
                    self.expression(loop_expression);
                }
                self.statement(&statement.body);
            }
            Statement::WhileStatement(statement) => {
                self.expression(&statement.condition);
                self.statement(&statement.body);
            }
            Statement::DoWhileStatement(statement) => {
                self.statement(&statement.body);
                self.expression(&statement.condition);
            }
            Statement::Return(statement) => {
                if let Some(expression) = &statement.expression {
                    self.expression(expression);
                }
            }
            Statement::VariableDeclarationStatement(statement) => {
                if let Some(initial_value) = &statement.initial_value {
                    self.expression(initial_value);
                }
                for declaration in statement.declarations.iter().flatten() {
                    self.type_name(&declaration.type_name);
                }
            }
            Statement::RevertStatement(statement) => self.function_call(&statement.error_call),
            Statement::EmitStatement(statement) => self.function_call(&statement.event_call),
            Statement::TryStatement(statement) => {
                self.expression(&statement.external_call);
                for clause in &statement.clauses {
                    self.statements(&clause.block.statements);
                }
            }
            Statement::UncheckedBlock(statement) => self.statements(&statement.statements),
            _ => {}
        }
    }

    /// Collects references from one expression.
    fn expression(&mut self, expression: &Expression) {
        match expression {
            Expression::FunctionCall(call) => self.function_call(call),
            Expression::Assignment(assignment) => {
                self.expression(&assignment.right_hand_side);
                self.expression(&assignment.left_hand_side);
            }
            Expression::MemberAccess(access) => {
                if let Some(id) = access.referenced_declaration {
                    self.add(id);
                }
                self.expression(&access.expression);
            }
            Expression::Identifier(identifier) => {
                if let Some(id) = identifier.referenced_declaration {
                    self.add(id);
                }
            }
            Expression::BinaryOperation(operation) => {
                self.expression(&operation.left_expression);
                self.expression(&operation.right_expression);
            }
            Expression::UnaryOperation(operation) => {
                self.expression(&operation.sub_expression);
            }
            Expression::Conditional(conditional) => {
                self.expression(&conditional.condition);
                self.expression(&conditional.true_expression);
                self.expression(&conditional.false_expression);
            }
            Expression::TupleExpression(tuple) => {
                for component in tuple.components.iter().flatten() {
                    self.expression(component);
                }
            }
            Expression::IndexAccess(access) => {
                self.expression(&access.base_expression);
                if let Some(index_expression) = &access.index_expression {
                    self.expression(index_expression);
                }
            }
            Expression::IndexRangeAccess(access) => {
                self.expression(&access.base_expression);
                if let Some(start) = &access.start_expression {
                    self.expression(start);
                }
            }
            _ => {}
        }
    }

    /// Collects symbols referenced inside a function call, including the
    /// called declaration and every argument expression. Chained calls such
    /// as `a().b().c()` descend into the inner call expressions.
    fn function_call(&mut self, call: &solc::ast::FunctionCall) {
        // 1. Resolve the callee and descend into chained calls.
        match &*call.expression {
            FunctionCallExpression::MemberAccess(access) => {
                if let Some(id) = access.referenced_declaration {
                    self.add(id);
                }
                self.expression(&access.expression);
            }
            FunctionCallExpression::Identifier(identifier) => {
                if let Some(id) = identifier.referenced_declaration {
                    // An unqualified call may be a virtual base hook, which
                    // dispatches to the most-derived override on the chain.
                    self.add_dispatchable(id);
                }
            }
            FunctionCallExpression::FunctionCallOptions(options) => {
                if let Some(id) = called_id(&options.expression) {
                    self.add(id);
                }
                self.expression(&options.expression);
                for option in &options.options {
                    self.expression(option);
                }
            }
            FunctionCallExpression::NewExpression(new_expression) => {
                if let TypeName::UserDefinedTypeName(defined) = &new_expression.type_name
                    && let Some(id) = defined.referenced_declaration
                {
                    self.add(id);
                }
            }
            _ => {}
        }

        // 2. Argument expressions.
        for argument in &call.arguments {
            self.expression(argument);
        }
    }

    /// Collects references from a type name.
    fn type_name(&mut self, type_name: &TypeName) {
        match type_name {
            TypeName::UserDefinedTypeName(defined) => {
                if let Some(id) = defined.referenced_declaration {
                    self.add(id);
                }
            }
            TypeName::ArrayTypeName(array) => self.type_name(&array.base_type),
            TypeName::Mapping(mapping) => {
                self.type_name(&mapping.key_type);
                self.type_name(&mapping.value_type);
            }
            TypeName::FunctionTypeName(function) => {
                for parameter in &function.parameter_types.parameters {
                    self.type_name(&parameter.type_name);
                }
                for parameter in &function.return_parameter_types.parameters {
                    self.type_name(&parameter.type_name);
                }
            }
            _ => {}
        }
    }
}

/// Collects the declaration ids referenced within the symbol source range.
fn collect_references(
    ast: &SourceUnit,
    symbol: &ResolvedSymbol,
    symbol_id: i64,
    index: &SymbolIndex,
    overrides: &OverrideTable,
) -> Vec<i64> {
    let range_start = symbol.offset;
    let range_end = symbol.offset.saturating_add(symbol.length);
    let mut walk = RefWalk {
        symbol_id,
        results: Vec::new(),
        seen_ids: HashSet::new(),
        index,
        overrides,
    };

    for node in &ast.nodes {
        match node {
            SourceUnitNode::ContractDefinition(contract) => {
                for member in &contract.nodes {
                    walk.member(member, range_start, range_end);
                }
            }
            SourceUnitNode::FunctionDefinition(function) => {
                walk.function(function, range_start, range_end);
            }
            SourceUnitNode::VariableDeclaration(variable) => {
                walk.variable(variable, range_start, range_end);
            }
            SourceUnitNode::StructDefinition(definition) => {
                walk.struct_definition(definition, range_start, range_end);
            }
            SourceUnitNode::ErrorDefinition(definition) => {
                walk.parameters(
                    &definition.parameters.parameters,
                    &definition.src,
                    range_start,
                    range_end,
                );
            }
            SourceUnitNode::EventDefinition(definition) => {
                walk.parameters(
                    &definition.parameters.parameters,
                    &definition.src,
                    range_start,
                    range_end,
                );
            }
            SourceUnitNode::UserDefinedValueTypeDefinition(definition) => {
                walk.value_type(definition, range_start, range_end);
            }
            _ => {}
        }
    }
    walk.results
}

/// Whether `src` intersects the `[range_start, range_end)` window.
fn overlaps(src: &SourceLocation, range_start: usize, range_end: usize) -> bool {
    src.offset < range_end && src.offset + src.length > range_start
}

/// Whether a modifier invocation is a base constructor specifier.
///
/// solc >= 0.8 sets the invocation kind, older compilers leave it absent, so
/// the specifier is inferred from the referenced declaration.
fn is_base_constructor(modifier: &ModifierInvocation, index: &SymbolIndex) -> bool {
    match modifier.kind {
        Some(ModifierInvocationKind::BaseConstructorSpecifier) => true,
        Some(_) => false,
        None => {
            let referenced = modifier
                .modifier_name
                .referenced_declaration
                .and_then(|id| index.entries.get(&id));
            match referenced {
                Some(entry) => entry.kind.is_contract(),
                None => false,
            }
        }
    }
}

/// Referenced declaration id of a called expression.
fn called_id(expression: &Expression) -> Option<i64> {
    match expression {
        Expression::MemberAccess(access) => access.referenced_declaration,
        Expression::Identifier(identifier) => identifier.referenced_declaration,
        _ => None,
    }
}

/// Resolves the Solidity function kind, inferring constructor and fallback
/// from older ASTs where `kind` is absent.
fn function_kind(function: &FunctionDefinition) -> FunctionKind {
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

/// Normalized parameter key of an AST parameter list.
fn ast_param_key(parameters: &[VariableDeclaration]) -> String {
    parameters
        .iter()
        .map(|parameter| normalize_type(&parameter.type_descriptions.type_string))
        .collect::<Vec<String>>()
        .join(",")
}

/// Normalizes Solc type strings, so both sides of a comparison drop the data
/// location, e.g. `struct Voter.Point memory` becomes
/// `struct Voter.ChainAllocationDispatch[]`.
fn normalize_type(kind: &Option<String>) -> String {
    let Some(kind) = kind else {
        return String::new();
    };
    kind.replace(" memory", "")
        .replace(" calldata", "")
        .replace(" storage", "")
        .replace(" payable", "")
}

/// Comma-separated readable parameter types of a declaration.
fn format_params(parameters: &[VariableDeclaration]) -> String {
    parameters
        .iter()
        .map(|parameter| format_type_name(&parameter.type_name))
        .collect::<Vec<String>>()
        .join(",")
}

/// Readable type of a declaration, e.g. `ChainAllocationDispatch[]`.
fn format_type_name(type_name: &TypeName) -> String {
    match type_name {
        TypeName::ElementaryTypeName(elementary) => match elementary.name {
            solc::ast::ElementaryType::Uint(bits) => format!("uint{bits}"),
            solc::ast::ElementaryType::Int(bits) => format!("int{bits}"),
            solc::ast::ElementaryType::Address => "address".to_owned(),
            solc::ast::ElementaryType::Payable => "address payable".to_owned(),
            solc::ast::ElementaryType::Bool => "bool".to_owned(),
            solc::ast::ElementaryType::String => "string".to_owned(),
            solc::ast::ElementaryType::Bytes => "bytes".to_owned(),
            solc::ast::ElementaryType::FixedBytes(bits) => format!("bytes{bits}"),
            solc::ast::ElementaryType::Ufixed(m, n) => format!("ufixed{m}x{n}"),
            solc::ast::ElementaryType::Fixed(m, n) => format!("fixed{m}x{n}"),
        },
        TypeName::ArrayTypeName(array) => format!("{}[]", format_type_name(&array.base_type)),
        TypeName::UserDefinedTypeName(defined) => {
            // checkrs: allow(clone_in_iterator)
            let path = defined.path_node.as_ref().map(|path| path.name.clone());
            path.or_else(|| defined.name.clone())
                .unwrap_or_else(|| "unknown".to_owned())
        }
        TypeName::Mapping(_) => "mapping".to_owned(),
        TypeName::FunctionTypeName(_) => "function".to_owned(),
    }
}

/// Natspec comment lines directly preceding a declaration offset.
fn extract_natspec(content: &str, offset: usize) -> String {
    let prefix = if offset > content.len() {
        content
    } else {
        &content[..offset]
    };
    let mut lines: Vec<&str> = Vec::new();
    for line in prefix.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("///") {
            lines.push(line);
        } else if trimmed.starts_with("/*") || trimmed.starts_with('*') {
            lines.push(line);
            if trimmed.starts_with("/*") {
                break;
            }
        } else if trimmed.is_empty() {
            lines.push(line);
        } else {
            break;
        }
    }
    lines.reverse();
    while let Some(last) = lines.last()
        && last.trim().is_empty()
    {
        lines.pop();
    }
    while let Some(first) = lines.first()
        && first.trim().is_empty()
    {
        lines.remove(0);
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// Resolves `@inheritdoc Contract` by copying the referenced natspec.
///
/// Returns the original natspec when the directive is absent or the
/// referenced declaration cannot be found.
fn resolve_inheritdoc(
    index: &SymbolIndex,
    sources: &HashMap<PathBuf, String>,
    symbol: &str,
    natspec: &str,
) -> String {
    // 1. Require an `/// @inheritdoc` directive.
    let Some(line) = natspec
        .lines()
        .find(|line| line.trim().starts_with("/// @inheritdoc"))
    else {
        return natspec.to_owned();
    };
    let rest = match line.trim().strip_prefix("/// @inheritdoc") {
        Some(rest) => rest.trim(),
        None => return natspec.to_owned(),
    };
    // The directive may reference a parent path like `Base.IActions`, only
    // the contract name itself matters.
    let contract_name = rest.rsplit('.').next().unwrap_or(rest);

    // 2. Function name of the symbol, e.g. `App.configure(address)`.
    let Some(after) = symbol.split('.').nth(1) else {
        return natspec.to_owned();
    };
    let function_name = after.split('(').next().unwrap_or("");
    if function_name.is_empty() {
        return natspec.to_owned();
    }

    // 3. Natspec of the matching declaration in the referenced contract.
    let Some(contract_ids) = index.contracts.get(contract_name) else {
        return natspec.to_owned();
    };
    for (contract_id, _) in contract_ids {
        let Some(entry) = index.member_function(*contract_id, function_name) else {
            continue;
        };
        let Some(content) = sources.get(&entry.file) else {
            continue;
        };
        let resolved = extract_natspec(content, entry.offset);
        if resolved.is_empty() {
            return natspec.to_owned();
        }
        return dedent(&resolved, base_indent(content, entry.offset));
    }
    natspec.to_owned()
}

/// Leading whitespace count on the line containing `offset`.
fn base_indent(content: &str, offset: usize) -> usize {
    let line_start = content[..offset.min(content.len())]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    content[line_start..offset.min(content.len())]
        .chars()
        .take_while(|character| character.is_whitespace())
        .count()
}

/// Strips up to `base` spaces of leading whitespace from every line.
fn dedent(text: &str, base: usize) -> String {
    if base == 0 {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line
            .chars()
            .take_while(|character| character.is_whitespace())
            .count()
            .min(base);
        result.push_str(&line[trimmed..]);
        result.push('\n');
    }
    result
}

/// The source text of a declaration range.
fn slice(content: &str, offset: usize, length: usize) -> String {
    let start = offset.min(content.len());
    let end = start.saturating_add(length).min(content.len());
    content[start..end].to_owned()
}

/// 1-based line number of a byte offset within a source file.
fn line_of(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count() + 1
}

/// Header of a contract definition, up to the opening brace.
fn contract_header(content: &str, offset: usize) -> &str {
    let remaining = &content[offset.min(content.len())..];
    match remaining.find('{') {
        Some(brace) => remaining[..brace].trim_end(),
        None => remaining.trim_end(),
    }
}

/// Display name of a symbol for section headings.
///
/// Functions and getters render their declaration name only, e.g.
/// `Voter.allocate(...)` becomes `allocate`. Other declarations keep their
/// symbol as-is.
fn display_name(symbol: &str, kind: &SymbolKind) -> String {
    match kind {
        SymbolKind::Function => {
            let after = symbol
                .split_once('.')
                .map(|(_, rest)| rest)
                .unwrap_or(symbol);
            match after.find('(') {
                Some(paren) => after[..paren].to_owned(),
                None => after.to_owned(),
            }
        }
        SymbolKind::Variable => symbol
            .split_once('.')
            .map(|(_, name)| name.to_owned())
            .unwrap_or_else(|| symbol.to_owned()),
        _ => symbol.to_owned(),
    }
}

/// Contract name of a `Contract.member` symbol.
fn contract_of(symbol: &str) -> &str {
    symbol.split('.').next().unwrap_or("?")
}
