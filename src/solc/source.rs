//! Source resolution for solc compilation.
//!
//! Walks the import graph starting from the harness target and returns every
//! reachable source keyed by its path relative to the project root. Sources
//! outside the root keep their absolute path. Each import is attempted in
//! order:
//!
//! 1. Relative to the importing file's directory
//! 2. Through the project remappings
//! 3. Relative to the current working directory

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::solc::RemappingsResolver;

/// Collects the transitive Solidity sources reachable from a target.
#[derive(Clone, Debug)]
pub struct SourceResolver {
    root: PathBuf,
    remappings: RemappingsResolver,
}

impl Default for SourceResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceResolver {
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("."),
            remappings: RemappingsResolver::default(),
        }
    }

    /// Sets the project root used to key the resolved sources.
    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        self.root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        self
    }

    pub fn with_remappings(mut self, remappings: RemappingsResolver) -> Self {
        self.remappings = remappings;
        self
    }

    pub fn solc_remappings(&self) -> Vec<String> {
        self.remappings.solc_remappings()
    }

    /// Reads `target` and its transitive imports.
    pub fn resolve(&self, target: impl AsRef<Path>) -> Result<HashMap<PathBuf, String>> {
        let target = target.as_ref();

        // 1. Seed the stack with the harness target.
        let mut sources = HashMap::new();
        let mut visited = HashSet::new();
        let mut stack = vec![target.to_path_buf()];

        while let Some(path) = stack.pop() {
            // 2. Canonicalize the path and skip already visited sources.
            let canonical = path
                .canonicalize()
                .unwrap_or_else(|_| normalize_path(&path));
            if visited.contains(&canonical) {
                continue;
            }
            // checkrs: allow(clone_in_loops)
            visited.insert(canonical.clone());

            // 3. Read the source, extract its imports, and key it relative
            //    to the project root.
            let content = fs::read_to_string(&canonical)
                .with_context(|| format!("failed to read {}", canonical.display()))?;
            let imports = extract_imports(&content);
            // checkrs: allow(clone_in_loops)
            let key = canonical
                .strip_prefix(&self.root)
                .unwrap_or(&canonical)
                .to_path_buf();
            sources.insert(key, content);

            let parent = canonical
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));

            // 4. Push every import that resolves to an existing file.
            for import in imports {
                // 4a. Relative to the importing file's directory.
                let import_path = parent.join(&import);
                let normalized = normalize_path(&import_path);
                if normalized.is_file() {
                    stack.push(normalized);
                    continue;
                }
                if let Ok(canonical_import) = import_path.canonicalize()
                    && canonical_import.is_file()
                {
                    stack.push(canonical_import);
                    continue;
                }

                // 4b. Through the project remappings.
                if let Some(remapped) = self.remappings.resolve(&import)
                    && remapped.is_file()
                {
                    stack.push(remapped);
                    continue;
                }

                // 4c. Relative to the current working directory.
                if Path::new(&import).is_file() {
                    stack.push(PathBuf::from(&import));
                }
            }
        }

        Ok(sources)
    }
}

fn strip_block_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_block = false;
    while let Some(c) = chars.next() {
        if !in_block && c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block = true;
            out.push(' ');
            out.push(' ');
            continue;
        }
        if in_block && c == '*' && chars.peek() == Some(&'/') {
            chars.next();
            in_block = false;
            out.push(' ');
            out.push(' ');
            continue;
        }
        if in_block {
            if c == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn extract_imports(content: &str) -> Vec<String> {
    let cleaned = strip_block_comments(content);
    let mut imports = Vec::new();
    for line in cleaned.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        for segment in trimmed.split(';') {
            let seg = segment.trim_start();
            if seg.is_empty() || seg.starts_with("//") {
                continue;
            }
            if !seg.starts_with("import") {
                continue;
            }
            let after = &seg["import".len()..];
            if !after.is_empty()
                && !after.starts_with(char::is_whitespace)
                && !after.starts_with('"')
                && !after.starts_with('\'')
                && !after.starts_with('{')
                && !after.starts_with('*')
            {
                continue;
            }
            let mut chars = seg.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '"' || ch == '\'' {
                    let quote = ch;
                    let mut path = String::new();
                    while let Some(&next) = chars.peek() {
                        if next == quote {
                            chars.next();
                            break;
                        }
                        path.push(next);
                        chars.next();
                    }
                    if path.ends_with(".sol") {
                        imports.push(path);
                    }
                }
            }
        }
    }
    imports
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut components: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if matches!(
                    components.last(),
                    Some(Component::Normal(_)) | Some(Component::ParentDir)
                ) {
                    components.pop();
                } else if !matches!(components.last(), Some(Component::RootDir)) {
                    components.push(comp);
                }
            }
            Component::CurDir => {}
            _ => components.push(comp),
        }
    }
    let mut out = PathBuf::new();
    for comp in components {
        out.push(comp.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_imports_simple() {
        let content = r#"import "./Support.sol"; import {Lib} from "./Lib.sol";"#;
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Support.sol", "./Lib.sol"]);
    }

    #[test]
    fn extract_imports_single_quotes() {
        let content = r"import './Foo.sol';";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Foo.sol"]);
    }

    #[test]
    fn extract_imports_no_imports() {
        let content = "contract Foo {}";
        let imports = extract_imports(content);
        assert!(imports.is_empty());
    }

    #[test]
    fn extract_imports_ignores_line_comment() {
        let content = "// import \"./Foo.sol\";\nimport \"./Bar.sol\";";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Bar.sol"]);
    }

    #[test]
    fn extract_imports_ignores_block_comment() {
        let content = "/* import \"./Foo.sol\"; */\nimport \"./Bar.sol\";";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Bar.sol"]);
    }

    #[test]
    fn extract_imports_ignores_import_with_prefix() {
        let content = "contract Foo {}\n  // import \"./Foo.sol\";\n/*\nimport \"./Bar.sol\";\n*/\nimport \"./Baz.sol\";";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Baz.sol"]);
    }

    #[test]
    fn normalize_path_cleans() {
        let p = PathBuf::from("a/./b/../c.sol");
        assert_eq!(normalize_path(&p), PathBuf::from("a/c.sol"));
    }
}
