# Rust Project Organization Guidelines

These guidelines are inspired by the Rust standard library and mature Rust crates.

The primary goal is:

> Organize code around domain concepts and types. Modules exist to support those
> concepts. Implementation details stay hidden.

* * *

## Core Philosophy

Think in terms of:

```rust
Project
Contract
ArtifactStore
Analyzer
Linker
```

not:

```rust
build_project(...)
load_artifacts(...)
analyze_contract(...)
```

A user of your crate should learn a small set of nouns.

Most operations should be discovered through those nouns.

* * *

## 1. Types Define the Public API

The public API should primarily consist of types.

Good:

```rust
Project
Contract
BuildOptions
Analyzer
```

Less desirable:

```rust
build_project(...)
load_contract(...)
run_analysis(...)
```

A useful question:

> What are the first five concepts a user should learn?

Those concepts should usually be types.

* * *

## 2. Organize Modules Around Domain Concepts

Good:

```text
src/
├── foundry/
│   ├── project.rs
│   ├── build.rs
│   └── artifacts.rs
├── evm/
│   ├── contract.rs
│   ├── linker.rs
│   └── analyzer.rs
```

Bad:

```text
src/
├── structs.rs
├── functions.rs
├── helpers.rs
├── utils.rs
```

A module should own a coherent responsibility.

Ask:

> What part of the domain does this module represent?

* * *

## 3. One Primary Type Per Module

A useful convention:

```text
project.rs  -> Project
contract.rs -> Contract
analyzer.rs -> Analyzer
linker.rs   -> Linker
```

The filename and primary exported type usually match.

When opening a file, it should be obvious what abstraction it exists to support.

* * *

## 4. Modules Exist to Support Types

Example:

```text
project/
├── mod.rs
├── build.rs
├── artifacts.rs
├── validation.rs
└── config.rs
```

```rust
pub struct Project {
    ...
}
```

All of these files exist because `Project` exists.

A useful test:

> If this type disappeared, would these modules still exist?

If not, they probably belong together.

* * *

## 5. Keep Implementation Details Private

Expose:

```rust
pub use project::Project;
pub use contract::Contract;
pub use build::BuildOptions;
```

Hide:

```rust
mod build;
mod validation;
mod loader;
mod resolver;
```

Consumers should think about:

```rust
Project
Contract
```

not:

```rust
validation
loader
resolver
```

* * *

## 6. Re-export Public Concepts

Prefer:

```rust
use crate::foundry::Project;
```

over:

```rust
use crate::foundry::project::Project;
```

Example:

```rust
// foundry/mod.rs

pub use project::Project;
pub use build::BuildOptions;

mod project;
mod build;
mod artifacts;
```

The public API should be smaller than the implementation.

* * *

## 7. Avoid Deep Hierarchies

Prefer:

```text
evm/
├── contract.rs
├── linker.rs
├── analyzer.rs
```

over:

```text
evm/
└── analysis/
    └── contract/
        └── linking/
            └── resolver.rs
```

Create nesting only when it introduces a meaningful abstraction boundary.

* * *

## 8. Avoid utils.rs

Whenever creating:

```text
utils.rs
helpers.rs
common.rs
```

ask:

> Which domain owns this functionality?

Prefer:

```text
project/config.rs
project/discovery.rs
evm/linker.rs
```

over:

```text
utils.rs
```

* * *

## 9. Separate Public Operations From Implementation

Expose operations through domain types:

```rust
let project = Project::open(path)?;
project.build(opts)?;

let artifacts = project.artifacts()?;
let contract = project.contract("Vault")?;
```

Hide implementation details:

```rust
build::compile_sources(...)
validation::verify_target(...)
artifacts::load_cache(...)
```

Users should see domain operations.

Implementation modules should remain internal.

* * *

## Method Design

## 10. Behavior Belongs to the Type That Owns the State

Good:

```rust
project.build()?;
project.artifacts()?;
contract.analyze()?;
```

Less desirable:

```rust
build_project(&project)?;
load_artifacts(&project)?;
analyze_contract(&contract)?;
```

A useful question:

> Does this operation primarily use data stored on this type?

If yes, it probably belongs as a method.

* * *

### 11. Constructors Are Entry Points

Prefer:

```rust
let project = Project::open(path)?;
```

over:

```rust
let project = open_project(path)?;
```

Users should discover behavior through types.

* * *

### 12. If The Function Name Starts With The Type Name, It May Be A Method

Bad smell:

```rust
build_project(...)
validate_project(...)
load_project(...)
```

Usually becomes:

```rust
project.build(...)
project.validate(...)
project.load(...)
```

Likewise:

```rust
contract_link(...)
contract_analyze(...)
```

becomes:

```rust
contract.link(...)
contract.analyze(...)
```

* * *

## 13. Free Functions Are Exceptions

Free functions are appropriate when there is no natural owner.

Examples:

```rust
parse_address(...)
decode_hex(...)
env::current_dir()
thread::sleep(...)
```

If a function naturally operates on a type, prefer a method.

* * *

## Handling Many Arguments

The most common challenge with methods is parameter explosion.

A useful rule:

> A method should primarily operate on state contained in `self`.

If most information comes from parameters rather than `self`, reconsider the design.

* * *

## 14. Use Option Types For Configuration

Instead of:

```rust
project.build(
    force,
    optimize,
    profile,
    cache,
)?;
```

Prefer:

```rust
project.build(
    BuildOptions::new()
        .force(true)
        .profile(Profile::Release),
)?;
```

```rust
pub struct BuildOptions {
    force: bool,
    optimize: bool,
    profile: Profile,
}
```

Common std examples:

```rust
OpenOptions
Command
DirBuilder
```

* * *

## 15. Move Stable Configuration Into The Type

Instead of:

```rust
project.build(force)?;
```

Use:

```rust
let project = Project {
    path,
    force,
};
```

Then:

```rust
project.build()?;
```

Configuration that is part of the object's identity should live on the object.

* * *

## 16. Introduce Operation Types For Complex Workflows

When an operation develops significant state:

```rust
pub struct Analyzer {
    config: Config,
}
```

```rust
Analyzer::new(config)
    .analyze(contract)?;
```

Or:

```rust
pub struct Linker {
    libraries: Libraries,
}
```

```rust
linker.link(contract)?;
```

Examples from the std ecosystem:

```rust
Command
OpenOptions
Builder
```

These are often called operation types or builder types.

* * *

## 17. Use Context Objects For Internal Workflows

Instead of:

```rust
fn analyze(
    contract: &Contract,
    artifacts: &Artifacts,
    config: &Config,
    cache: &Cache,
    findings: &mut Findings,
)
```

Prefer:

```rust
pub struct AnalysisContext {
    artifacts: Artifacts,
    config: Config,
    cache: Cache,
}
```

```rust
analyzer.analyze(contract, &ctx)?;
```

This prevents parameter explosion.

* * *

## 18. If Most Inputs Are Not `self`, It Might Not Be A Method

Consider:

```rust
contract.link(
    artifacts,
    resolver,
    libraries,
    config,
)
```

Almost all information comes from elsewhere.

This may be better as:

```rust
linker.link(contract)?;
```

or:

```rust
Linker::new(config)
    .link(contract)?;
```

A useful test:

> If I removed `self`, would the function still need almost all the same arguments?

If yes, the operation may belong elsewhere.

* * *

## Design Decision Tree

When adding functionality:

### Step 1

Ask:

> Is there an existing type that owns this behavior?

If yes:

```rust
impl Type {
    fn method(...)
}
```

### Step 2

Ask:

> Does this introduce a new domain concept?

If yes:

```rust
struct Analyzer;
struct Linker;
struct Builder;
```

Create a new type and module.

### Step 3

Ask:

> Is there no natural owner?

If yes:

```rust
parse_address(...)
decode_hex(...)
```

Use a free function.

### Step 4

Ask:

> Does this operation need substantial configuration?

If yes:

```rust
BuildOptions
AnalysisOptions
LinkOptions
```

Introduce an options type.

### Step 5

Ask:

> Does this operation have significant state or behavior of its own?

If yes:

```rust
Analyzer
Linker
Builder
```

Introduce an operation type.

### Step 6

Ask:

> Would removing `self` change very little about the function?

If yes, it probably should not be a method.

## Public API Litmus Test

A good API reads like this:

```rust
let project = Project::open(path)?;
project.build(BuildOptions::release())?;

let contract = project.contract("Vault")?;

Analyzer::new(config)
    .analyze(&contract)?;
```

The user learns:

```rust
Project
Contract
BuildOptions
Analyzer
```

The user does not need to learn:

```rust
build
loader
validation
resolver
helpers
utils
```

The larger the codebase grows, the more valuable this distinction becomes.
