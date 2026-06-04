//! Coverage reporter that generates an lcov.info file from build artifacts and
//! shared coverage data.
//!
//! [`CoverageReporter`] is the entry point. It takes two inputs:
//!
//! 1. **Build artifacts** - compiled Solidity artifacts (parsed from `out/*.json`)
//!    that contain deployed bytecode and source maps.
//! 2. [`SharedCoverage`] - raw per-PC hit counts collected during fuzzing.
//!
//! The reporter resolves every hit PC back to a source line using the artifact
//! source maps, then aggregates the results into a single `lcov.info` report.
//!
//! # Source ID resolution
//!
//! Foundry incremental builds can shift source unit IDs between compilation
//! units. To resolve a source map `file_id` without relying on unstable global
//! build-info files, the reporter scans every artifact and builds a local
//! `source_id -> path` map for each source file. When a source ID is not found
//! in the primary artifact's local map, the reporter recursively checks the
//! maps of all transitively imported artifacts.
//!
//! # Usage
//!
//! ```text
//! let report = CoverageReporter::new()
//!     .build_artifacts(artifacts)
//!     .shared_coverage(shared_coverage)
//!     .build();
//! let lcov_info = format!("{report}");
//! ```
//!
//! # Definitions
//!
//! ## Root Artifact
//!
//! A **Root Artifact** is any artifact whose deployed bytecode hash appears in
//! the [`SharedCoverage`] data.
//!
//! ## Child Artifact
//!
//! A **Child Artifact** is a resolved artifact based on a Root Artifact's
//! `metadata.sources`. Child artifacts can have their own child artifacts and
//! must be resolved recursively.
//!
//! ## Report's Source Files
//!
//! Every source file path listed in `metadata.sources` of a resolved artifact
//! must appear in the final report.
//!
//! ## Executable vs Non Executable Lines
//!
//! Executable and non-executable lines are derived from the resolved
//! artifact's AST nodes. A single node may span multiple lines.
//!
//! | AST Node                       | Executable |
//! |--------------------------------|------------|
//! | `PragmaDirective`              | No         |
//! | `ImportDirective`              | No         |
//! | `ContractDefinition`           | No         |
//! | `VariableDeclaration`          | No         |
//! | `ExpressionStatement`          | Yes        |
//! | `VariableDeclarationStatement` | Yes        |
//! | `IfStatement`                  | Yes        |
//! | `ForStatement`                 | Yes        |
//! | `WhileStatement`               | Yes        |
//! | `DoWhileStatement`             | Yes        |
//! | `Return`                       | Yes        |
//! | `EmitStatement`                | Yes        |
//! | `RevertStatement`              | Yes        |
//! | `TryStatement`                 | Yes        |
//! | `InlineAssembly`               | Yes        |
//! | `Break`                        | Yes        |
//! | `Continue`                     | Yes        |
//!
//! All executable lines have a hit count of **0** by default. Only lines that
//! actually receive a coverage hit are updated with a positive count.
//!
//! # Function Hit Count
//!
//! The hit count of a function is derived from the maximum **direct** statement
//! child. A function's direct children are the top-level executable statements
//! in its body.
//!
//! # Statement Hit Count
//!
//! A compound statement's hit count is the number of times it is entered. For
//! `ForStatement`, this is the hit count of the initialization expression. For
//! `IfStatement`, this is the hit count of the condition expression. The hit
//! count of a leaf statement is the maximum line hit across its source range.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::PathBuf;

use alloy_primitives::{B256, keccak256};

use crate::evm::coverage::shared::SharedCoverage;
use crate::evm::coverage::source_map::{SourceMapEntry, parse_source_map};
use crate::foundry::{Artifact, ArtifactId, LinkReferences};

// ---------------------------------------------------------------------------
// Bytecode helpers
// ---------------------------------------------------------------------------

fn collect_link_positions(link_refs: &LinkReferences) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for libs in link_refs.values() {
        for refs in libs.values() {
            for r in refs {
                out.push((r.start, r.length));
            }
        }
    }
    out
}

fn zero_out_positions(buf: &mut [u8], positions: &[(usize, usize)]) {
    for (start, len) in positions {
        for i in *start..*start + *len {
            if i < buf.len() {
                buf[i] = 0;
            }
        }
    }
}

fn parse_bytecode_with_placeholders(object: &str, link_refs: &LinkReferences) -> Vec<u8> {
    let hex = object.strip_prefix("0x").unwrap_or(object);
    let mut hex_positions = Vec::new();
    for libs in link_refs.values() {
        for refs in libs.values() {
            for r in refs {
                hex_positions.push((r.start * 2, r.length * 2));
            }
        }
    }
    hex_positions.sort_by_key(|(start, _)| *start);
    let mut cleaned = String::new();
    let mut last_end = 0;
    for (start, len) in hex_positions {
        cleaned.push_str(&hex[last_end..start]);
        cleaned.push_str(&"00".repeat(len / 2));
        last_end = start + len;
    }
    cleaned.push_str(&hex[last_end..]);
    hex::decode(cleaned).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// PC-to-source map
// ---------------------------------------------------------------------------

fn build_pc_to_source_map(
    bytecode: &[u8],
    source_map: &[SourceMapEntry],
) -> Vec<Option<SourceMapEntry>> {
    let mut result = vec![None; bytecode.len()];
    let mut pc = 0;
    let mut opcode_idx = 0;
    while pc < bytecode.len() {
        if let Some(entry) = source_map.get(opcode_idx) {
            result[pc] = Some(*entry);
        }
        let opcode = bytecode[pc];
        let push_size = if (0x60..=0x7f).contains(&opcode) {
            (opcode - 0x60 + 1) as usize
        } else {
            0
        };
        pc += 1 + push_size;
        opcode_idx += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Source ID resolver
// ---------------------------------------------------------------------------

struct SourceIdResolver {
    local_maps: HashMap<PathBuf, HashMap<usize, PathBuf>>,
    imports: HashMap<PathBuf, Vec<PathBuf>>,
}

impl SourceIdResolver {
    fn new(artifacts: &[Artifact]) -> Self {
        let mut local_maps = HashMap::new();
        let mut imports: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let mut source_cache: HashMap<PathBuf, String> = HashMap::new();

        // Pre-read source files and build direct imports map.
        for artifact in artifacts {
            // checkrs: allow(clone_in_loops)
            let artifact_path = artifact.ast().absolute_path.clone();
            let full_path = artifact.project_path().join(&artifact_path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                source_cache.insert(artifact_path.clone(), content); // checkrs: allow(clone_in_loops)
            }

            let mut artifact_imports = Vec::new();
            for node in &artifact.ast().nodes {
                let solc::ast::SourceUnitNode::ImportDirective(import) = node else {
                    continue;
                };
                // checkrs: allow(clone_in_loops)
                let imported_path = import.absolute_path.clone();
                let imported_full = artifact.project_path().join(&imported_path);
                if let Ok(content) = fs::read_to_string(&imported_full) {
                    source_cache.insert(imported_path.clone(), content); // checkrs: allow(clone_in_loops)
                }
                artifact_imports.push(imported_path);
            }
            imports.insert(artifact_path, artifact_imports);
        }

        for artifact in artifacts {
            // checkrs: allow(clone_in_loops)
            let artifact_path = artifact.ast().absolute_path.clone();
            let mut local = HashMap::new();

            // The artifact's own source_id -> path.
            local.insert(artifact.source_id(), artifact_path.clone()); // checkrs: allow(clone_in_loops)

            // Collect source map entries from both bytecode and deployed bytecode.
            let mut source_map_entries: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
            for sm in [
                artifact.bytecode().map(|b| b.source_map.as_str()),
                artifact.deployed_bytecode().map(|b| b.source_map.as_str()),
            ]
            .into_iter()
            .flatten()
            {
                for entry in parse_source_map(sm) {
                    source_map_entries
                        .entry(entry.source_index)
                        .or_default()
                        .push((entry.offset, entry.length));
                }
            }

            // Gather candidate source files (self + transitive imports).
            let mut candidates: HashSet<PathBuf> = HashSet::new();
            candidates.insert(artifact_path.clone()); // checkrs: allow(clone_in_loops)
            let mut visited = HashSet::new();
            let mut queue = vec![artifact_path.clone()];
            while let Some(current) = queue.pop() {
                // checkrs: allow(clone_in_loops)
                if !visited.insert(current.clone()) {
                    continue;
                }
                // checkrs: allow(nested_if_let)
                if let Some(direct_imports) = imports.get(&current) {
                    for imported in direct_imports {
                        // checkrs: allow(clone_in_loops)
                        if candidates.insert(imported.clone()) {
                            queue.push(imported.clone()); // checkrs: allow(clone_in_loops)
                        }
                    }
                }
            }

            // Try to match unknown source IDs to candidate files.
            // checkrs: allow(clone_in_loops)
            let mut unknown_source_ids: Vec<usize> = source_map_entries
                .keys()
                .copied()
                .filter(|id| !local.contains_key(id))
                .collect();

            let mut fits: HashMap<usize, Vec<PathBuf>> = HashMap::new();
            for sid in &unknown_source_ids {
                let Some(entries) = source_map_entries.get(sid) else {
                    continue;
                };
                for candidate in &candidates {
                    if let Some(content) = source_cache.get(candidate)
                        && entries
                            .iter()
                            .all(|(offset, length)| offset.saturating_add(*length) <= content.len())
                    {
                        fits.entry(*sid).or_default().push(candidate.clone()); // checkrs: allow(clone_in_loops)
                    }
                }
            }

            // Greedy assignment: sort by number of fits (ascending) and assign
            // when a unique remaining candidate exists.
            unknown_source_ids.sort_by_key(|sid| fits.get(sid).map(|v| v.len()).unwrap_or(0));

            for sid in unknown_source_ids {
                let Some(candidates) = fits.get(&sid) else {
                    continue;
                };
                let available: Vec<PathBuf> = candidates
                    .iter()
                    .filter(|c| !local.values().any(|v| v == *c))
                    .cloned()
                    .collect();
                if available.len() == 1 {
                    local.insert(sid, available[0].clone()); // checkrs: allow(clone_in_loops)
                }
            }

            // checkrs: allow(clone_in_loops)
            local_maps.insert(artifact_path, local);
        }

        Self {
            local_maps,
            imports,
        }
    }

    fn resolve(&self, artifact: &Artifact, source_id: usize) -> Option<PathBuf> {
        let start = artifact.ast().absolute_path.clone();
        let mut visited = HashSet::new();
        self.resolve_recursive(&start, source_id, &mut visited)
    }

    fn resolve_recursive(
        &self,
        current: &PathBuf,
        source_id: usize,
        visited: &mut HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        if !visited.insert(current.clone()) {
            return None;
        }

        if let Some(map) = self.local_maps.get(current)
            && let Some(path) = map.get(&source_id)
        {
            return Some(path.clone());
        }

        if let Some(direct_imports) = self.imports.get(current) {
            for imported in direct_imports {
                if let Some(path) = self.resolve_recursive(imported, source_id, visited) {
                    return Some(path);
                }
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Artifact index
// ---------------------------------------------------------------------------

struct ArtifactIndexEntry<'a> {
    artifact: &'a Artifact,
    hash: B256,
    positions: Vec<(usize, usize)>,
    is_initcode: bool,
}

struct ArtifactIndex<'a> {
    entries: Vec<ArtifactIndexEntry<'a>>,
}

impl<'a> ArtifactIndex<'a> {
    fn new(artifacts: &'a [Artifact]) -> Self {
        let mut entries = Vec::new();
        for artifact in artifacts {
            // Index deployed bytecode.
            let deployed_entry = artifact.deployed_bytecode().and_then(|deployed| {
                let code =
                    parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
                if code.is_empty() {
                    return None;
                }
                let mut positions = collect_link_positions(&deployed.link_references);
                for refs in deployed.immutable_references.values() {
                    for r in refs {
                        positions.push((r.start, r.length));
                    }
                }
                if matches!(artifact, Artifact::Library(_)) && code.first() == Some(&0x73) {
                    positions.push((1, 20));
                }
                let mut masked = code;
                zero_out_positions(&mut masked, &positions);
                let hash = keccak256(&masked);
                Some(ArtifactIndexEntry {
                    artifact,
                    hash,
                    positions,
                    is_initcode: false,
                })
            });
            if let Some(entry) = deployed_entry {
                entries.push(entry);
            }

            // Index initcode (bytecode) for constructor coverage.
            let initcode_entry = artifact.bytecode().and_then(|bytecode| {
                let code =
                    parse_bytecode_with_placeholders(&bytecode.object, &bytecode.link_references);
                if code.is_empty() {
                    return None;
                }
                let positions = collect_link_positions(&bytecode.link_references);
                let mut masked = code;
                zero_out_positions(&mut masked, &positions);
                let hash = keccak256(&masked);
                Some(ArtifactIndexEntry {
                    artifact,
                    hash,
                    positions,
                    is_initcode: true,
                })
            });
            if let Some(entry) = initcode_entry {
                entries.push(entry);
            }
        }
        Self { entries }
    }

    fn find(&self, raw_bytecode: &[u8]) -> Option<(&'a Artifact, bool)> {
        let mut masked = raw_bytecode.to_vec();
        for entry in &self.entries {
            zero_out_positions(&mut masked, &entry.positions);
            if keccak256(&masked) == entry.hash {
                return Some((entry.artifact, entry.is_initcode));
            }
            // Restore masked bytes for the next entry.
            for (start, len) in &entry.positions {
                for i in *start..*start + *len {
                    if i < raw_bytecode.len() {
                        masked[i] = raw_bytecode[i];
                    }
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Source range hits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct ArtifactSourceHits {
    hits: BTreeMap<(usize, usize), u64>,
}

impl ArtifactSourceHits {
    fn add(&mut self, source_index: usize, offset: usize, hit: u64) {
        let key = (source_index, offset);
        let entry = self.hits.entry(key).or_insert(0);
        *entry = (*entry).max(hit);
    }

    fn max_in_range(&self, source_index: usize, offset: usize, length: usize) -> u64 {
        self.hits
            .range((source_index, offset)..(source_index, offset + length))
            .map(|(_, hit)| *hit)
            .max()
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Expression source helpers
// ---------------------------------------------------------------------------

fn expr_src(expr: &solc::ast::Expression) -> &solc::ast::SourceLocation {
    match expr {
        solc::ast::Expression::Assignment(e) => &e.src,
        solc::ast::Expression::BinaryOperation(e) => &e.src,
        solc::ast::Expression::Conditional(e) => &e.src,
        solc::ast::Expression::ElementaryTypeNameExpression(e) => &e.src,
        solc::ast::Expression::FunctionCall(e) => &e.src,
        solc::ast::Expression::Identifier(e) => &e.src,
        solc::ast::Expression::IndexAccess(e) => &e.src,
        solc::ast::Expression::IndexRangeAccess(e) => &e.src,
        solc::ast::Expression::Literal(e) => &e.src,
        solc::ast::Expression::MemberAccess(e) => &e.src,
        solc::ast::Expression::NewExpression(e) => &e.src,
        solc::ast::Expression::TupleExpression(e) => &e.src,
        solc::ast::Expression::UnaryOperation(e) => &e.src,
        solc::ast::Expression::VariableDeclarationStatement(e) => &e.src,
        solc::ast::Expression::ExpressionStatement(e) => &e.src,
    }
}

// ---------------------------------------------------------------------------
// Source content resolution
// ---------------------------------------------------------------------------

fn resolve_content(
    artifact: &Artifact,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    src: &solc::ast::SourceLocation,
) -> Option<(PathBuf, String)> {
    let path = resolver.resolve(artifact, src.source_index)?;
    let content = source_cache.get(&path).cloned().unwrap_or_else(|| {
        let full_path = artifact.project_path().join(&path);
        fs::read_to_string(&full_path).unwrap_or_default()
    });
    if content.is_empty() {
        return None;
    }
    Some((path, content))
}

// ---------------------------------------------------------------------------
// Statement hit computation
// ---------------------------------------------------------------------------

/// Return the maximum hit count for a source range.
///
/// First checks `source_hits` for the exact range. If that yields zero, falls
/// back to `line_hits` so that inherited functions whose bytecode was executed
/// by a child contract are still reported correctly.
fn max_hit_from_src(
    artifact: &Artifact,
    source_hits: &ArtifactSourceHits,
    line_hits: &HashMap<PathBuf, HashMap<usize, u64>>,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    src: &solc::ast::SourceLocation,
) -> u64 {
    let hit = source_hits.max_in_range(src.source_index, src.offset, src.length);
    if hit > 0 {
        return hit;
    }
    let Some((path, content)) = resolve_content(artifact, resolver, source_cache, src) else {
        return 0;
    };
    let empty = HashMap::new();
    let file_hits = line_hits.get(&path).unwrap_or(&empty);
    let start_line = offset_to_line(&content, src.offset);
    let end_line = offset_to_line(&content, src.offset + src.length);
    file_hits
        .iter()
        .filter(|(line, _)| **line >= start_line && **line <= end_line)
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0)
}

fn compute_statement_hit(
    stmt: &solc::ast::Statement,
    artifact: &Artifact,
    source_hits: &ArtifactSourceHits,
    line_hits: &HashMap<PathBuf, HashMap<usize, u64>>,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
) -> u64 {
    match stmt {
        solc::ast::Statement::Block(block) => block
            .statements
            .iter()
            .map(|s| {
                compute_statement_hit(s, artifact, source_hits, line_hits, resolver, source_cache)
            })
            .max()
            .unwrap_or(0),
        solc::ast::Statement::UncheckedBlock(block) => block
            .statements
            .iter()
            .map(|s| {
                compute_statement_hit(s, artifact, source_hits, line_hits, resolver, source_cache)
            })
            .max()
            .unwrap_or(0),
        solc::ast::Statement::IfStatement(if_stmt) => {
            let cond_src = expr_src(if_stmt.condition.as_ref());
            max_hit_from_src(
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
                cond_src,
            )
        }
        solc::ast::Statement::ForStatement(for_stmt) => {
            let entry_hit = for_stmt
                .initialization_expression
                .as_ref()
                .map(|expr| {
                    let src = expr_src(expr.as_ref());
                    max_hit_from_src(
                        artifact,
                        source_hits,
                        line_hits,
                        resolver,
                        source_cache,
                        src,
                    )
                })
                .unwrap_or(0);
            let cond_src = expr_src(for_stmt.condition.as_ref());
            let cond_hit = max_hit_from_src(
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
                cond_src,
            );
            if entry_hit > 0 { entry_hit } else { cond_hit }
        }
        solc::ast::Statement::WhileStatement(while_stmt) => {
            let cond_src = expr_src(while_stmt.condition.as_ref());
            max_hit_from_src(
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
                cond_src,
            )
        }
        solc::ast::Statement::DoWhileStatement(do_while_stmt) => {
            let cond_src = expr_src(do_while_stmt.condition.as_ref());
            max_hit_from_src(
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
                cond_src,
            )
        }
        solc::ast::Statement::TryStatement(try_stmt) => {
            let call_src = expr_src(try_stmt.external_call.as_ref());
            let call_hit = max_hit_from_src(
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
                call_src,
            );
            let clauses_hit = try_stmt
                .clauses
                .iter()
                .map(|c| {
                    c.block
                        .statements
                        .iter()
                        .map(|s| {
                            compute_statement_hit(
                                s,
                                artifact,
                                source_hits,
                                line_hits,
                                resolver,
                                source_cache,
                            )
                        })
                        .max()
                        .unwrap_or(0)
                })
                .max()
                .unwrap_or(0);
            call_hit.max(clauses_hit)
        }
        _ => {
            let src = stmt_src(stmt);
            let Some((path, content)) = resolve_content(artifact, resolver, source_cache, src)
            else {
                return 0;
            };
            let empty = HashMap::new();
            let file_hits = line_hits.get(&path).unwrap_or(&empty);
            let start_line = offset_to_line(&content, src.offset);
            let end_line = offset_to_line(&content, src.offset + src.length);
            file_hits
                .iter()
                .filter(|(line, _)| **line >= start_line && **line <= end_line)
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(0)
        }
    }
}

// ---------------------------------------------------------------------------
// Statement line hit helpers
// ---------------------------------------------------------------------------

fn add_statement_line_hits(
    artifact: &Artifact,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    src: &solc::ast::SourceLocation,
    hit: u64,
    statement_line_hits: &mut HashMap<PathBuf, HashMap<usize, u64>>,
) {
    let Some(path) = resolver.resolve(artifact, src.source_index) else {
        return;
    };
    let content = source_cache.get(&path).cloned().unwrap_or_else(|| {
        let full_path = artifact.project_path().join(&path);
        fs::read_to_string(&full_path).unwrap_or_default()
    });
    if content.is_empty() {
        return;
    }
    let start_line = offset_to_line(&content, src.offset);
    let end_line = offset_to_line(&content, src.offset + src.length);
    let hits = statement_line_hits.entry(path).or_default();
    for line in start_line..=end_line {
        hits.insert(line, hit);
    }
}

fn collect_statement_line_hits(
    stmt: &solc::ast::Statement,
    artifact: &Artifact,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    source_hits: &ArtifactSourceHits,
    line_hits: &HashMap<PathBuf, HashMap<usize, u64>>,
    statement_line_hits: &mut HashMap<PathBuf, HashMap<usize, u64>>,
) {
    match stmt {
        solc::ast::Statement::Block(block) => {
            for stmt in &block.statements {
                collect_statement_line_hits(
                    stmt,
                    artifact,
                    resolver,
                    source_cache,
                    source_hits,
                    line_hits,
                    statement_line_hits,
                );
            }
        }
        solc::ast::Statement::UncheckedBlock(block) => {
            for stmt in &block.statements {
                collect_statement_line_hits(
                    stmt,
                    artifact,
                    resolver,
                    source_cache,
                    source_hits,
                    line_hits,
                    statement_line_hits,
                );
            }
        }
        solc::ast::Statement::IfStatement(if_stmt) => {
            // Only process children; do not set the if statement lines.
            // The if line already gets its hit from the condition PC, and
            // body lines are overridden by their respective statements.
            collect_statement_line_hits(
                &if_stmt.true_body,
                artifact,
                resolver,
                source_cache,
                source_hits,
                line_hits,
                statement_line_hits,
            );
            if let Some(false_body) = &if_stmt.false_body {
                collect_statement_line_hits(
                    false_body,
                    artifact,
                    resolver,
                    source_cache,
                    source_hits,
                    line_hits,
                    statement_line_hits,
                );
            }
        }
        solc::ast::Statement::ForStatement(for_stmt) => {
            let stmt_hit = compute_statement_hit(
                stmt,
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
            );
            add_statement_line_hits(
                artifact,
                resolver,
                source_cache,
                &for_stmt.src,
                stmt_hit,
                statement_line_hits,
            );
            collect_statement_line_hits(
                &for_stmt.body,
                artifact,
                resolver,
                source_cache,
                source_hits,
                line_hits,
                statement_line_hits,
            );
        }
        solc::ast::Statement::WhileStatement(while_stmt) => {
            let stmt_hit = compute_statement_hit(
                stmt,
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
            );
            add_statement_line_hits(
                artifact,
                resolver,
                source_cache,
                &while_stmt.src,
                stmt_hit,
                statement_line_hits,
            );
            collect_statement_line_hits(
                &while_stmt.body,
                artifact,
                resolver,
                source_cache,
                source_hits,
                line_hits,
                statement_line_hits,
            );
        }
        solc::ast::Statement::DoWhileStatement(do_while_stmt) => {
            let stmt_hit = compute_statement_hit(
                stmt,
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
            );
            add_statement_line_hits(
                artifact,
                resolver,
                source_cache,
                &do_while_stmt.src,
                stmt_hit,
                statement_line_hits,
            );
            collect_statement_line_hits(
                &do_while_stmt.body,
                artifact,
                resolver,
                source_cache,
                source_hits,
                line_hits,
                statement_line_hits,
            );
        }
        solc::ast::Statement::TryStatement(try_stmt) => {
            let stmt_hit = compute_statement_hit(
                stmt,
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
            );
            add_statement_line_hits(
                artifact,
                resolver,
                source_cache,
                &try_stmt.src,
                stmt_hit,
                statement_line_hits,
            );
            for clause in &try_stmt.clauses {
                for stmt in &clause.block.statements {
                    collect_statement_line_hits(
                        stmt,
                        artifact,
                        resolver,
                        source_cache,
                        source_hits,
                        line_hits,
                        statement_line_hits,
                    );
                }
            }
        }
        _ => {
            let stmt_hit = compute_statement_hit(
                stmt,
                artifact,
                source_hits,
                line_hits,
                resolver,
                source_cache,
            );
            add_statement_line_hits(
                artifact,
                resolver,
                source_cache,
                stmt_src(stmt),
                stmt_hit,
                statement_line_hits,
            );
        }
    }
}

fn collect_statement_line_hits_from_artifacts(
    artifacts: &[&Artifact],
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    source_hits: &HashMap<&ArtifactId, ArtifactSourceHits>,
    line_hits: &HashMap<PathBuf, HashMap<usize, u64>>,
) -> HashMap<PathBuf, HashMap<usize, u64>> {
    let mut statement_line_hits: HashMap<PathBuf, HashMap<usize, u64>> = HashMap::new();

    for artifact in artifacts {
        let ast = artifact.ast();
        for node in &ast.nodes {
            match node {
                solc::ast::SourceUnitNode::ContractDefinition(contract) => {
                    for node in &contract.nodes {
                        if let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node
                            && let Some(body) = &func.body
                        {
                            for stmt in &body.statements {
                                collect_statement_line_hits(
                                    stmt,
                                    artifact,
                                    resolver,
                                    source_cache,
                                    source_hits
                                        .get(artifact.id())
                                        .unwrap_or(&ArtifactSourceHits::default()),
                                    line_hits,
                                    &mut statement_line_hits,
                                );
                            }
                        }
                    }
                }
                solc::ast::SourceUnitNode::FunctionDefinition(func) => {
                    if let Some(body) = &func.body {
                        for stmt in &body.statements {
                            collect_statement_line_hits(
                                stmt,
                                artifact,
                                resolver,
                                source_cache,
                                source_hits
                                    .get(artifact.id())
                                    .unwrap_or(&ArtifactSourceHits::default()),
                                line_hits,
                                &mut statement_line_hits,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    statement_line_hits
}

// ---------------------------------------------------------------------------
// Line offset helpers
// ---------------------------------------------------------------------------

fn offset_to_line(content: &str, offset: usize) -> usize {
    let safe_offset = offset.min(content.len());
    let mut line = content
        .bytes()
        .take(safe_offset)
        .filter(|&b| b == b'\n')
        .count()
        + 1;
    if safe_offset == content.len() && content.ends_with('\n') {
        line = line.saturating_sub(1);
    }
    line
}

// ---------------------------------------------------------------------------
// Function coverage helpers
// ---------------------------------------------------------------------------

fn collect_functions_from_artifacts(
    artifacts: &[&Artifact],
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    source_hits: &HashMap<&ArtifactId, ArtifactSourceHits>,
    line_hits: &HashMap<PathBuf, HashMap<usize, u64>>,
) -> HashMap<PathBuf, Vec<FunctionCoverage>> {
    let mut file_functions: HashMap<PathBuf, HashMap<String, (usize, u64)>> = HashMap::new();

    for artifact in artifacts {
        let ast = artifact.ast();
        let mut collect = |func: &solc::ast::FunctionDefinition| {
            let name = match func.kind {
                solc::ast::FunctionKind::Constructor => "constructor".to_string(),
                solc::ast::FunctionKind::Fallback => "fallback".to_string(),
                solc::ast::FunctionKind::Receive => "receive".to_string(),
                _ if func.name.is_empty() => return,
                _ => func.name.clone(), // checkrs: allow(clone_in_loops)
            };
            let Some(path) = resolver.resolve(artifact, func.src.source_index) else {
                return;
            };
            let content = source_cache.get(&path).cloned().unwrap_or_else(|| {
                let full_path = artifact.project_path().join(&path);
                fs::read_to_string(&full_path).unwrap_or_default()
            });
            if content.is_empty() {
                return;
            }
            let start_line = offset_to_line(&content, func.src.offset);

            let source_hits = source_hits.get(artifact.id()).cloned().unwrap_or_default();

            let func_hits = func.body.as_ref().map_or(0, |body| {
                let stmt_hits = body
                    .statements
                    .iter()
                    .map(|stmt| {
                        compute_statement_hit(
                            stmt,
                            artifact,
                            &source_hits,
                            line_hits,
                            resolver,
                            source_cache,
                        )
                    })
                    .max()
                    .unwrap_or(0);
                if stmt_hits > 0 {
                    stmt_hits
                } else {
                    // Fall back to the raw line hit for the function start line.
                    line_hits
                        .get(&path)
                        .and_then(|hits| hits.get(&start_line).copied())
                        .unwrap_or(0)
                }
            });

            file_functions
                .entry(path)
                .or_default()
                .insert(name, (start_line, func_hits)); // checkrs: allow(clone_in_loops)
        };

        for node in &ast.nodes {
            match node {
                solc::ast::SourceUnitNode::FunctionDefinition(func) => collect(func),
                solc::ast::SourceUnitNode::ContractDefinition(contract) => {
                    for node in &contract.nodes {
                        if let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node {
                            collect(func);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    file_functions
        .into_iter()
        .map(|(path, funcs)| {
            let functions: Vec<FunctionCoverage> = funcs
                .into_iter()
                .map(|(name, (line, hits))| FunctionCoverage { name, line, hits })
                .collect();
            (path, functions)
        })
        .collect()
}

/// Return the [`SourceLocation`] of a [`Statement`].
fn stmt_src(stmt: &solc::ast::Statement) -> &solc::ast::SourceLocation {
    match stmt {
        solc::ast::Statement::Block(block) => &block.src,
        solc::ast::Statement::Break(b) => &b.src,
        solc::ast::Statement::Continue(c) => &c.src,
        solc::ast::Statement::DoWhileStatement(stmt) => &stmt.src,
        solc::ast::Statement::EmitStatement(stmt) => &stmt.src,
        solc::ast::Statement::ExpressionStatement(stmt) => &stmt.src,
        solc::ast::Statement::ForStatement(stmt) => &stmt.src,
        solc::ast::Statement::IfStatement(stmt) => &stmt.src,
        solc::ast::Statement::InlineAssembly(stmt) => &stmt.src,
        solc::ast::Statement::PlaceholderStatement(stmt) => &stmt.src,
        solc::ast::Statement::Return(stmt) => &stmt.src,
        solc::ast::Statement::RevertStatement(stmt) => &stmt.src,
        solc::ast::Statement::TryStatement(stmt) => &stmt.src,
        solc::ast::Statement::UncheckedBlock(block) => &block.src,
        solc::ast::Statement::VariableDeclarationStatement(stmt) => &stmt.src,
        solc::ast::Statement::WhileStatement(stmt) => &stmt.src,
    }
}

/// Recursively add all lines spanned by a [`SourceLocation`] to the executable
/// line set.
fn add_executable_lines_from_src(
    artifact: &Artifact,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    src: &solc::ast::SourceLocation,
    executable_lines: &mut HashMap<PathBuf, HashSet<usize>>,
) {
    let Some(path) = resolver.resolve(artifact, src.source_index) else {
        return;
    };
    let content = source_cache.get(&path).cloned().unwrap_or_else(|| {
        let full_path = artifact.project_path().join(&path);
        fs::read_to_string(&full_path).unwrap_or_default()
    });
    if content.is_empty() {
        return;
    }
    let start_line = offset_to_line(&content, src.offset);
    let end_line = offset_to_line(&content, src.offset + src.length);
    for line in start_line..=end_line {
        executable_lines
            .entry(path.clone()) // checkrs: allow(clone_in_loops)
            .or_default()
            .insert(line);
    }
}

/// Recursively collect executable lines from a [`Statement`] and its children.
fn collect_executable_lines_from_statement(
    stmt: &solc::ast::Statement,
    artifact: &Artifact,
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
    executable_lines: &mut HashMap<PathBuf, HashSet<usize>>,
) {
    match stmt {
        solc::ast::Statement::Block(block) => {
            for stmt in &block.statements {
                collect_executable_lines_from_statement(
                    stmt,
                    artifact,
                    resolver,
                    source_cache,
                    executable_lines,
                );
            }
        }
        solc::ast::Statement::UncheckedBlock(block) => {
            for stmt in &block.statements {
                collect_executable_lines_from_statement(
                    stmt,
                    artifact,
                    resolver,
                    source_cache,
                    executable_lines,
                );
            }
        }
        solc::ast::Statement::IfStatement(if_stmt) => {
            add_executable_lines_from_src(
                artifact,
                resolver,
                source_cache,
                &if_stmt.src,
                executable_lines,
            );
            collect_executable_lines_from_statement(
                &if_stmt.true_body,
                artifact,
                resolver,
                source_cache,
                executable_lines,
            );
            if let Some(false_body) = &if_stmt.false_body {
                collect_executable_lines_from_statement(
                    false_body,
                    artifact,
                    resolver,
                    source_cache,
                    executable_lines,
                );
            }
        }
        solc::ast::Statement::ForStatement(for_stmt) => {
            add_executable_lines_from_src(
                artifact,
                resolver,
                source_cache,
                &for_stmt.src,
                executable_lines,
            );
            collect_executable_lines_from_statement(
                &for_stmt.body,
                artifact,
                resolver,
                source_cache,
                executable_lines,
            );
        }
        solc::ast::Statement::WhileStatement(while_stmt) => {
            add_executable_lines_from_src(
                artifact,
                resolver,
                source_cache,
                &while_stmt.src,
                executable_lines,
            );
            collect_executable_lines_from_statement(
                &while_stmt.body,
                artifact,
                resolver,
                source_cache,
                executable_lines,
            );
        }
        solc::ast::Statement::DoWhileStatement(do_while_stmt) => {
            add_executable_lines_from_src(
                artifact,
                resolver,
                source_cache,
                &do_while_stmt.src,
                executable_lines,
            );
            collect_executable_lines_from_statement(
                &do_while_stmt.body,
                artifact,
                resolver,
                source_cache,
                executable_lines,
            );
        }
        solc::ast::Statement::TryStatement(try_stmt) => {
            add_executable_lines_from_src(
                artifact,
                resolver,
                source_cache,
                &try_stmt.src,
                executable_lines,
            );
            for clause in &try_stmt.clauses {
                for stmt in &clause.block.statements {
                    collect_executable_lines_from_statement(
                        stmt,
                        artifact,
                        resolver,
                        source_cache,
                        executable_lines,
                    );
                }
            }
        }
        _ => {
            // All other statement types are executable.
            add_executable_lines_from_src(
                artifact,
                resolver,
                source_cache,
                stmt_src(stmt),
                executable_lines,
            );
        }
    }
}

/// Collect executable lines from the AST of all resolved artifacts.
fn collect_executable_lines_from_artifacts(
    artifacts: &[&Artifact],
    resolver: &SourceIdResolver,
    source_cache: &HashMap<PathBuf, String>,
) -> HashMap<PathBuf, HashSet<usize>> {
    let mut executable_lines: HashMap<PathBuf, HashSet<usize>> = HashMap::new();

    for artifact in artifacts {
        let ast = artifact.ast();
        for node in &ast.nodes {
            match node {
                solc::ast::SourceUnitNode::ContractDefinition(contract) => {
                    for node in &contract.nodes {
                        if let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node
                            && let Some(body) = &func.body
                        {
                            for stmt in &body.statements {
                                collect_executable_lines_from_statement(
                                    stmt,
                                    artifact,
                                    resolver,
                                    source_cache,
                                    &mut executable_lines,
                                );
                            }
                        }
                    }
                }
                solc::ast::SourceUnitNode::FunctionDefinition(func) => {
                    if let Some(body) = &func.body {
                        for stmt in &body.statements {
                            collect_executable_lines_from_statement(
                                stmt,
                                artifact,
                                resolver,
                                source_cache,
                                &mut executable_lines,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    executable_lines
}

// ---------------------------------------------------------------------------
// Coverage report
// ---------------------------------------------------------------------------

/// A coverage report in lcov.info format.
#[derive(Debug, Clone, Default)]
pub struct CoverageReport {
    files: Vec<FileCoverage>,
}

#[derive(Debug, Clone)]
struct FunctionCoverage {
    name: String,
    line: usize,
    hits: u64,
}

#[derive(Debug, Clone)]
struct FileCoverage {
    path: PathBuf,
    line_hits: HashMap<usize, u64>,
    functions: Vec<FunctionCoverage>,
}

impl CoverageReport {
    /// Compute the overall line coverage percentage across all files.
    pub fn coverage(&self) -> f64 {
        let mut total = 0usize;
        let mut hit = 0usize;
        for file in &self.files {
            total += file.line_hits.len();
            hit += file.line_hits.values().filter(|&&h| h > 0).count();
        }
        if total > 0 {
            (hit as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }
}

impl fmt::Display for CoverageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for file in &self.files {
            writeln!(f, "TN:")?;
            writeln!(f, "SF:{}", file.path.display())?;

            let mut functions: Vec<&FunctionCoverage> = file.functions.iter().collect();
            functions.sort_by(|a, b| {
                let ord = a.line.cmp(&b.line);
                if ord == std::cmp::Ordering::Equal {
                    a.name.cmp(&b.name)
                } else {
                    ord
                }
            });

            for func in &functions {
                writeln!(f, "FN:{},{}", func.line, func.name)?;
            }
            for func in &functions {
                writeln!(f, "FNDA:{},{}", func.hits, func.name)?;
            }
            let fnf = functions.len();
            let fnh = functions.iter().filter(|f| f.hits > 0).count();
            if fnf > 0 {
                writeln!(f, "FNF:{}", fnf)?;
                writeln!(f, "FNH:{}", fnh)?;
            }

            let mut lines: Vec<(&usize, &u64)> = file.line_hits.iter().collect();
            lines.sort_by_key(|(line, _)| *line);

            for (line, hits) in &lines {
                writeln!(f, "DA:{},{}", line, hits)?;
            }

            let lf = lines.len();
            let lh = lines.iter().filter(|(_, hits)| **hits > 0).count();

            writeln!(f, "LF:{}", lf)?;
            writeln!(f, "LH:{}", lh)?;
            writeln!(f, "end_of_record")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Coverage reporter
// ---------------------------------------------------------------------------

/// Orchestrates the building of lcov coverage reports.
#[derive(Debug, Clone)]
pub struct CoverageReporter {
    artifacts: Vec<Artifact>,
    shared_coverage: SharedCoverage,
}

impl Default for CoverageReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverageReporter {
    /// Create a new empty [`CoverageReporter`].
    pub fn new() -> Self {
        Self {
            artifacts: Vec::new(),
            shared_coverage: SharedCoverage::new(),
        }
    }

    /// Set the build artifacts.
    pub fn build_artifacts(mut self, artifacts: Vec<Artifact>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Set the [`SharedCoverage`] data.
    pub fn shared_coverage(mut self, coverage: SharedCoverage) -> Self {
        self.shared_coverage = coverage;
        self
    }

    /// Build the coverage report.
    ///
    /// 1. Resolve root artifacts (bytecode in [`SharedCoverage`]) and their
    ///    child artifacts recursively via `metadata.sources`.
    /// 2. Collect executable lines from the AST of every resolved artifact.
    /// 3. Map raw PC hits to source lines using the artifact source maps.
    /// 4. Assemble the final `lcov.info` report.
    pub fn build(self) -> CoverageReport {
        let resolver = SourceIdResolver::new(&self.artifacts);
        let index = ArtifactIndex::new(&self.artifacts);

        // -------------------------------------------------------------------
        // 1. Resolve root and child artifacts.
        //
        // Root artifacts are those whose deployed bytecode appears in the
        // coverage data. Child artifacts are resolved recursively from each
        // root artifact's `metadata.sources`.
        // -------------------------------------------------------------------
        let mut path_to_artifact: HashMap<PathBuf, &Artifact> = HashMap::new();
        for artifact in &self.artifacts {
            // checkrs: allow(clone_in_loops)
            path_to_artifact.insert(artifact.ast().absolute_path.clone(), artifact);
        }

        let mut resolved_source_paths: HashSet<PathBuf> = HashSet::new();
        let mut resolved_artifacts: HashSet<&ArtifactId> = HashSet::new();
        let mut queue: Vec<&Artifact> = Vec::new();

        let all_counts = self.shared_coverage.all_raw_edge_counts_with_bytecodes();
        for counts in &all_counts {
            let Some((artifact, _is_initcode)) = index.find(&counts.bytecode) else {
                continue;
            };
            if resolved_artifacts.insert(artifact.id()) {
                queue.push(artifact);
            }
        }

        while let Some(artifact) = queue.pop() {
            match artifact.metadata_sources() {
                Some(sources) => {
                    for path in sources.keys() {
                        let path = PathBuf::from(path);
                        // checkrs: allow(clone_in_loops)
                        let inserted = resolved_source_paths.insert(path.clone());
                        if inserted
                            && let Some(child) = path_to_artifact.get(&path)
                            && resolved_artifacts.insert(child.id())
                        {
                            queue.push(child);
                        }
                    }
                }
                None => {
                    // checkrs: allow(clone_in_loops)
                    resolved_source_paths.insert(artifact.ast().absolute_path.clone());
                }
            }
        }

        let has_active_filter = !resolved_source_paths.is_empty();

        // -------------------------------------------------------------------
        // 2. Collect executable lines from the AST of resolved artifacts.
        //
        // Executable lines are determined by AST statement nodes, not by the
        // deployed-bytecode source map. This ensures every executable line is
        // reported even when the compiler omits it from the source map.
        // -------------------------------------------------------------------
        let mut source_cache: HashMap<PathBuf, String> = HashMap::new();

        let resolved_artifact_refs: Vec<&Artifact> = self
            .artifacts
            .iter()
            .filter(|a| resolved_artifacts.contains(a.id()))
            .collect();

        let mut executable_lines = collect_executable_lines_from_artifacts(
            &resolved_artifact_refs,
            &resolver,
            &source_cache,
        );

        // -------------------------------------------------------------------
        // 3. Collect line hits from the shared coverage map.
        // -------------------------------------------------------------------
        let mut line_hits: HashMap<PathBuf, HashMap<usize, u64>> = HashMap::new();
        let mut source_hits: HashMap<&ArtifactId, ArtifactSourceHits> = HashMap::new();

        let all_counts = self.shared_coverage.all_raw_edge_counts_with_bytecodes();
        for counts in all_counts {
            let Some((artifact, is_initcode)) = index.find(&counts.bytecode) else {
                tracing::debug!(
                    "unmatched bytecode: len={}, id={:?}",
                    counts.bytecode.len(),
                    counts.contract_id
                );
                continue;
            };
            tracing::debug!(
                "matched artifact: {} (bytecode len={}, initcode={})",
                artifact.id(),
                counts.bytecode.len(),
                is_initcode
            );
            let (code, source_map) = if is_initcode {
                let Some(bytecode) = artifact.bytecode() else {
                    continue;
                };
                let code =
                    parse_bytecode_with_placeholders(&bytecode.object, &bytecode.link_references);
                let source_map = parse_source_map(&bytecode.source_map);
                (code, source_map)
            } else {
                let Some(deployed) = artifact.deployed_bytecode() else {
                    continue;
                };
                let code =
                    parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
                let source_map = parse_source_map(&deployed.source_map);
                (code, source_map)
            };
            let pc_map = build_pc_to_source_map(&code, &source_map);

            // checkrs: allow(clone_in_loops)
            for (pc, raw_count) in counts.raw_edges.iter().enumerate() {
                if *raw_count == 0 {
                    continue;
                }
                let Some(entry) = pc_map.get(pc).copied().flatten() else {
                    continue;
                };
                let Some(path) = resolver.resolve(artifact, entry.source_index) else {
                    continue;
                };
                let project_path = artifact.project_path();
                let full_path = project_path.join(&path);
                let content = source_cache
                    .entry(path.clone()) // checkrs: allow(clone_in_loops)
                    .or_insert_with(|| fs::read_to_string(&full_path).unwrap_or_default());
                if content.is_empty() {
                    continue;
                }
                let line = offset_to_line(content, entry.offset);
                let file_hits = line_hits.entry(path).or_default();
                let current = file_hits.entry(line).or_insert(0);
                *current = (*current).max(*raw_count);

                let hits = source_hits.entry(artifact.id()).or_default();
                hits.add(entry.source_index, entry.offset, *raw_count);
            }
        }

        // -------------------------------------------------------------------
        // 3.5. Adjust line hits with statement-level coverage.
        // -------------------------------------------------------------------
        let statement_line_hits = collect_statement_line_hits_from_artifacts(
            &resolved_artifact_refs,
            &resolver,
            &source_cache,
            &source_hits,
            &line_hits,
        );

        for (path, stmt_hits) in &statement_line_hits {
            // checkrs: allow(clone_in_loops)
            let file_hits = line_hits.entry(path.clone()).or_default();
            for (line, hit) in stmt_hits {
                file_hits.insert(*line, *hit);
            }
        }

        // -------------------------------------------------------------------
        // 4. Collect function coverage.
        // -------------------------------------------------------------------
        let mut file_functions = collect_functions_from_artifacts(
            &resolved_artifact_refs,
            &resolver,
            &source_cache,
            &source_hits,
            &line_hits,
        );

        // Ensure every function start line has a corresponding DA entry.
        for (path, functions) in &file_functions {
            let lines = executable_lines.entry(path.clone()).or_default(); // checkrs: allow(clone_in_loops)
            let hits = line_hits.entry(path.clone()).or_default(); // checkrs: allow(clone_in_loops)
            for func in functions {
                lines.insert(func.line);
                hits.entry(func.line)
                    .and_modify(|e| *e = (*e).max(func.hits))
                    .or_insert(func.hits);
            }
        }

        // Ensure all resolved source files appear in the report.
        for path in &resolved_source_paths {
            // checkrs: allow(clone_in_loops)
            executable_lines.entry(path.clone()).or_default();
        }

        // -------------------------------------------------------------------
        // 5. Build the report.
        // -------------------------------------------------------------------
        let mut files = Vec::new();
        for (path, lines) in executable_lines {
            if has_active_filter && !resolved_source_paths.contains(&path) {
                continue;
            }
            let hits = line_hits.remove(&path).unwrap_or_default();
            let functions = file_functions.remove(&path).unwrap_or_default();
            let mut file_coverage = FileCoverage {
                path,
                line_hits: HashMap::new(),
                functions,
            };
            for line in lines {
                let count = hits.get(&line).copied().unwrap_or(0);
                file_coverage.line_hits.insert(line, count);
            }
            files.push(file_coverage);
        }

        // Add any remaining hit-only files (should not normally happen, but be safe).
        for (path, hits) in line_hits {
            if has_active_filter && !resolved_source_paths.contains(&path) {
                continue;
            }
            let functions = file_functions.remove(&path).unwrap_or_default();
            files.push(FileCoverage {
                path,
                line_hits: hits,
                functions,
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        CoverageReport { files }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use alloy_primitives::U256;
    use alloy_sol_types::SolCall;
    use revm::primitives::{Address, Bytes};

    use crate::evm::coverage::exec::{ExecutionContractCoverage, ExecutionCoverage};
    use crate::evm::{
        Chain, ChainConfig, Contract, DeployInput, SetupInput, SharedCoverage, Transaction,
    };
    use crate::foundry;

    use super::*;

    alloy_sol_types::sol! {
        interface CoverageBranch {
            function branch(bool take) external;
        }

        interface TargetContract {
            function inheritanceCall(uint256 a) external returns (uint256);
            function libCall(uint256 amount) external returns (uint256);
            function libLinkedCall(uint256 amount) external returns (uint256);
            function interfaceCall(uint256 amount) external returns (uint256);
            function counterLinked() external returns (address);
        }

        interface TargetContractBasic {
            function addAndSub(uint256 a, uint256 b) external returns (uint256);
        }

        interface TargetContractWithLoop {
            function runLoop(uint256 count) external;
            function runNestedLoop(uint256 outer, uint256 inner) external;
        }

        interface EmptyTargetFunction {
            function dummyTargetFunction() external;
        }

        interface InheritedTarget {
            function inheritedTargetFunction() external;
        }

        interface CoverageInactiveUser {
            function callUsed() external pure returns (uint256);
        }
    }

    fn load_coverage_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    struct Deployed {
        chain: Chain,
        address: Address,
        global: SharedCoverage,
    }

    fn deploy_and_setup(contract: &Contract) -> Deployed {
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let mut deploy_opts = DeployInput::new(&contract.initcode);
        for lib in &contract.libraries {
            deploy_opts = deploy_opts.add_library(lib.clone()); // checkrs: allow(clone_in_loops)
        }
        let deployment = chain.deploy(deploy_opts).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let global = SharedCoverage::new();
        global.merge(&deployment.coverage);

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = SetupInput::new(target).calldata(setup_data);
            let setup = chain.setup(setup_opts).unwrap();
            assert!(setup.result.success, "setup must succeed");
            global.merge(&setup.coverage);
        }

        Deployed {
            chain,
            address: target,
            global,
        }
    }

    fn build_report(shared_coverage: &SharedCoverage, artifacts: &[Artifact]) -> CoverageReport {
        CoverageReporter::new()
            .build_artifacts(artifacts.to_vec())
            .shared_coverage(shared_coverage.clone())
            .build()
    }

    fn project_path() -> PathBuf {
        fs::canonicalize("fixtures/target-contract-coverage")
            .unwrap_or_else(|_| PathBuf::from("fixtures/target-contract-coverage"))
    }

    /// Regression test: build artifacts that include interfaces (which have
    /// no deployed bytecode) must not cause coverage report generation to fail.
    #[test]
    fn coverage_report_build_with_interface_artifact() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        assert!(
            !report.files.is_empty(),
            "coverage report should be generated even when build artifacts include interfaces"
        );
    }

    /// Regression test: lines executed once must display a hit count of 1.
    #[test]
    fn target_contract_basic_call_once() {
        let contract = load_coverage_fixture("src/TargetContractBasic.sol:TargetContractBasic");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                    .abi_encode(),
            )),
        ];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file =
            "fixtures/target-contract-coverage/expected/TargetContractBasicOnce.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: lines executed twice must display a hit count of 2.
    #[test]
    fn target_contract_basic_call_twice() {
        let contract = load_coverage_fixture("src/TargetContractBasic.sol:TargetContractBasic");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractBasic::addAndSubCall::new((U256::from(123), U256::from(123)))
                    .abi_encode(),
            )),
        ];

        let exec1 = deployed.chain.exec(&txs).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage1);

        let exec2 = deployed.chain.exec(&txs).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage2);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file =
            "fixtures/target-contract-coverage/expected/TargetContractBasicTwice.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: coverage must be correctly reported for contracts
    /// that contain loops in constructor, setup, and target functions.
    #[test]
    fn target_contract_with_loop() {
        let contract =
            load_coverage_fixture("src/TargetContractWithLoop.sol:TargetContractWithLoop");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithLoop::runLoopCall::new((U256::from(3),)).abi_encode(),
            )),
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithLoop::runNestedLoopCall::new((U256::from(2), U256::from(2)))
                    .abi_encode(),
            )),
        ];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file =
            "fixtures/target-contract-coverage/expected/TargetContractWithLoop.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: internal library coverage must be correctly reported,
    /// including the active contract that uses the library.
    #[test]
    fn coverage_report_lib_call() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::libCallCall::new((U256::from(123),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file = "fixtures/target-contract-coverage/expected/libCall.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: linked library coverage must be correctly reported,
    /// including the active contract that uses the linked library.
    #[test]
    fn coverage_report_lib_linked_call() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::libLinkedCallCall::new((U256::from(123),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file = "fixtures/target-contract-coverage/expected/libLinkedCall.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: a deployed contract must be reported correctly even
    /// when the caller interacts with it through an interface.
    #[test]
    fn coverage_report_interface_call() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::interfaceCallCall::new((U256::from(123),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file = "fixtures/target-contract-coverage/expected/interfaceCall.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: coverage report generation must not crash and must
    /// produce non-empty output even when the target function body is empty.
    #[test]
    fn coverage_report_empty_target_function() {
        let contract = load_coverage_fixture("src/EmptyTargetFunction.sol:EmptyTargetFunction");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            EmptyTargetFunction::dummyTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file = "fixtures/target-contract-coverage/expected/emptyTargetFunction.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    #[test]
    fn coverage_report_inherited_target_function() {
        let contract = load_coverage_fixture("src/InheritedTarget.sol:InheritedTarget");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            InheritedTarget::inheritedTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        assert!(
            !report.files.is_empty(),
            "coverage report must be generated for a target function inherited from a base contract"
        );
    }

    /// Regression test: genhtml requires a DA entry for every FN line. If a
    /// function's start line is not in the source map, the coverage report must
    /// still emit a DA entry for that line so genhtml does not fail with
    /// "unexpected category UNK".
    #[test]
    fn coverage_report_function_start_line_without_source_map() {
        let contract = load_coverage_fixture("src/UnusedLibraryUser.sol:UnusedLibraryUser");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            hex::decode("771602f7").unwrap(), // useAdd(uint256,uint256)
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        // The report must contain a DA entry for the unused library function
        // start line (line 9) even if the library's deployed bytecode source
        // map does not include it.
        let unused_library = report
            .files
            .iter()
            .find(|f| f.path.ends_with("UnusedLibrary.sol"))
            .expect("UnusedLibrary.sol must be in report");
        assert!(
            unused_library.line_hits.contains_key(&9),
            "UnusedLibrary.sol must contain DA entry for unused function at line 9: {unused_library:?}"
        );
    }

    /// Regression test: a source map entry whose offset points to the end of a
    /// source file that ends with a newline must not produce a line number beyond
    /// the file's actual line count.
    #[test]
    fn coverage_report_trailing_newline_no_out_of_range() {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let mut artifacts: Vec<Artifact> =
            project.load_artifacts().unwrap().into_values().collect();

        // Extract the deployed bytecode from the artifact before we modify it.
        let mut bytecode = Vec::new();
        for artifact in &artifacts {
            if artifact.id().to_string()
                == "src/CoverageTrailingNewline.sol:CoverageTrailingNewline"
            {
                bytecode = artifact.deployed_bytecode().unwrap().to_bytes().to_vec();
            }
        }
        assert!(!bytecode.is_empty(), "deployed bytecode must not be empty");

        // Inject a fake source map entry pointing to the end of the file to
        // simulate a compiler-generated entry that sits past the final newline.
        let source_path =
            PathBuf::from("fixtures/target-contract-coverage/src/CoverageTrailingNewline.sol");
        let content = fs::read_to_string(&source_path).unwrap();
        let file_len = content.len();

        for artifact in &mut artifacts {
            if artifact.id().to_string()
                == "src/CoverageTrailingNewline.sol:CoverageTrailingNewline"
            {
                if let Artifact::Contract(a) = artifact {
                    let original = a.deployed_bytecode.source_map.clone();
                    a.deployed_bytecode.source_map =
                        format!("{}:0:{}:-:0;{}", file_len, a.source_id, original);
                }
            }
        }

        // Create a fake coverage hit for PC 0 (the first opcode) which maps to the fake entry.
        let global = SharedCoverage::new();
        let mut fake_local = ExecutionCoverage::new();
        let mut fake_contract = ExecutionContractCoverage::new(bytecode.len());
        fake_contract.bytecode = bytecode;
        fake_contract.edges[0] = 1;
        fake_contract.hit_pcs.push(0);
        let contract_id = keccak256(&fake_contract.bytecode);
        fake_local.contracts.insert(contract_id, fake_contract);
        global.merge(&fake_local);

        let report = build_report(&global, &artifacts);

        let file = report
            .files
            .iter()
            .find(|f| f.path.ends_with("CoverageTrailingNewline.sol"))
            .expect("CoverageTrailingNewline.sol must be in report");
        let max_line = *file.line_hits.keys().max().unwrap_or(&0);
        let line_count = content.lines().count();
        assert!(
            max_line <= line_count,
            "CoverageTrailingNewline.sol must not contain a line number beyond the file's line count ({line_count}), but found {max_line}"
        );
    }

    /// Regression test: contracts with immutable variables must be matched
    /// correctly by the coverage reporter so that their coverage is not lost.
    #[test]
    fn coverage_report_immutable_contract_matched() {
        let contract = load_coverage_fixture("src/CoverageImmutable.sol:CoverageImmutable");
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let deploy_opts = DeployInput::new(&contract.initcode);
        let deployment = chain.deploy(deploy_opts).unwrap();
        assert!(deployment.result.success, "deployment must succeed");

        let global = SharedCoverage::new();
        global.merge(&deployment.coverage);

        let txs = vec![
            Transaction::new(deployment.address.unwrap()).calldata(Bytes::from(
                hex::decode("20965255").unwrap(), // getValue()
            )),
        ];
        let exec = chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&global, &artifacts);

        let file = report
            .files
            .iter()
            .find(|f| f.path.ends_with("CoverageImmutable.sol"))
            .expect("CoverageImmutable.sol must be in report");
        let get_value_line = 15; // line of function getValue() signature
        let body_line = 16; // line of function getValue() body
        assert!(
            file.line_hits.contains_key(&get_value_line),
            "CoverageImmutable.sol must contain DA entry for getValue() line {get_value_line}: {file:?}"
        );
        assert!(
            file.line_hits.get(&get_value_line).unwrap_or(&0) > &0,
            "CoverageImmutable.sol getValue() line {get_value_line} must have hits > 0: {file:?}"
        );
        assert!(
            file.line_hits.contains_key(&body_line),
            "CoverageImmutable.sol must contain DA entry for getValue() body line {body_line}: {file:?}"
        );
        assert!(
            file.line_hits.get(&body_line).unwrap_or(&0) > &0,
            "CoverageImmutable.sol getValue() body line {body_line} must have hits > 0: {file:?}"
        );
    }

    /// Regression test: inactive artifacts must not contribute executable lines
    /// to the coverage report. Only artifacts whose bytecode was recorded during
    /// fuzzing should define the set of executable source lines.
    /// Regression test: inactive artifacts must not contribute executable lines
    /// to the coverage report. Only artifacts whose bytecode was recorded during
    /// fuzzing should define the set of executable source lines.
    #[test]
    fn coverage_report_inactive_artifact_no_executable_lines() {
        let contract = load_coverage_fixture("src/CoverageInactiveUser.sol:CoverageInactiveUser");
        let mut deployed = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            CoverageInactiveUser::callUsedCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        let file = report
            .files
            .iter()
            .find(|f| f.path.ends_with("CoverageInactive.sol"))
            .expect("CoverageInactive.sol must be in report");

        // The active contract only inlines usedFunction (lines 8-9). The inactive
        // library artifact CoverageInactive has a source map covering line 7
        // (library declaration). Without the fix, line 7 would be added to
        // executable_lines by the inactive artifact and appear in the report.
        assert!(
            !file.line_hits.contains_key(&7),
            "CoverageInactive.sol must not contain a DA entry for library declaration line 7 contributed by inactive artifact: {file:?}"
        );
    }
}
