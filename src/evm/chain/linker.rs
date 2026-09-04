//! Solidity library linker.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alloy_primitives::Address;
use anyhow::{Context, Result, ensure};
use solc::ContractOutput;

use crate::compilers::solc::SolcOutput;
use crate::evm::chain::DeployLibraryInput;

/// Linker operation type that replaces Solidity library placeholders in
/// initcode with deployed addresses.
pub struct Linker;

impl Linker {
    /// Compute the Solidity placeholder string for a library identifier.
    ///
    /// The placeholder format is `__$<keccak256(identifier)[:34]>$__`.
    pub fn get_library_placeholder(identifier: &str) -> String {
        let hash = alloy_primitives::keccak256(identifier.as_bytes());
        let hex = alloy_primitives::hex::encode(hash);
        format!("__${}$__", &hex[..34])
    }

    /// Replace library placeholders in initcode with deployed addresses.
    pub fn link_libraries(initcode: &str, libraries: &HashMap<String, Address>) -> String {
        let mut hex = initcode.to_owned();
        for (identifier, address) in libraries {
            let placeholder = Self::get_library_placeholder(identifier);
            let address_hex = hex::encode(address);
            hex = hex.replace(&placeholder, &address_hex);
        }
        hex
    }

    /// Resolve the linked libraries of the compiled target contract into
    /// deployable inputs.
    ///
    /// Libraries with only internal functions are inlined by solc and leave
    /// no link references, so they resolve to an empty list.
    pub fn resolve_libraries(solc_output: &SolcOutput) -> Result<Vec<DeployLibraryInput>> {
        let Some(link_references) = solc_output
            .contract()?
            .evm
            .as_ref()
            .and_then(|evm| evm.bytecode.as_ref())
            .and_then(|bytecode| bytecode.link_references.as_ref())
        else {
            return Ok(Vec::new());
        };

        let mut libraries = Vec::new();
        for (file, names) in link_references {
            for name in names.keys() {
                libraries.push(Self::resolve_library(
                    &solc_output.output.contracts,
                    file,
                    name,
                )?);
            }
        }
        Ok(libraries)
    }

    /// Resolve one library and its nested dependencies into a deployable
    /// input.
    ///
    /// The `file:name` identifier matches the placeholder solc embeds in the
    /// linking bytecode.
    fn resolve_library(
        contracts: &HashMap<PathBuf, HashMap<String, ContractOutput>>,
        file: &str,
        name: &str,
    ) -> Result<DeployLibraryInput> {
        let identifier = format!("{file}:{name}");
        let contract = contracts
            .get(Path::new(file))
            .with_context(|| format!("library source `{file}` not found in compilation output"))?
            .get(name)
            .with_context(|| format!("library `{identifier}` not found in compilation output"))?;
        let bytecode = contract
            .evm
            .as_ref()
            .and_then(|evm| evm.bytecode.as_ref())
            .with_context(|| {
                format!("library `{identifier}` initcode missing from compilation output")
            })?;
        let initcode = bytecode.object.as_ref().with_context(|| {
            format!("library `{identifier}` initcode missing from compilation output")
        })?;
        ensure!(
            !initcode.is_empty(),
            "library `{identifier}` has empty initcode"
        );

        let mut library = DeployLibraryInput::new(identifier, initcode);
        for (nested_file, nested_names) in bytecode.link_references.iter().flatten() {
            for nested_name in nested_names.keys() {
                library = library.add_library(Self::resolve_library(
                    contracts,
                    nested_file,
                    nested_name,
                )?);
            }
        }
        Ok(library)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::keccak256;

    use crate::harness::HarnessId;

    use super::*;

    fn solc_output(contracts: serde_json::Value) -> SolcOutput {
        SolcOutput {
            id: HarnessId::try_from("src/Contract.sol:Contract").unwrap(),
            output: serde_json::from_value(serde_json::json!({"contracts": contracts})).unwrap(),
        }
    }

    fn placeholder(identifier: &str) -> String {
        let hash = keccak256(identifier.as_bytes());
        format!("__${}$__", &alloy_primitives::hex::encode(hash)[..34])
    }

    #[test]
    fn placeholder_uses_keccak_prefix() {
        assert_eq!(
            placeholder("MathLib.sol:MathLib"),
            Linker::get_library_placeholder("MathLib.sol:MathLib")
        );
        assert_eq!(
            Linker::get_library_placeholder("MathLib.sol:MathLib").len(),
            40
        );
    }

    #[test]
    fn link_libraries_replaces_placeholders() {
        let identifier = "MathLib.sol:MathLib";
        let mut libraries = HashMap::new();
        libraries.insert(identifier.to_string(), Address::new([0xab; 20]));

        let linked =
            Linker::link_libraries(&format!("0x73{}6000", placeholder(identifier)), &libraries);

        assert_eq!(linked, format!("0x73{}6000", "ab".repeat(20)));
    }

    #[test]
    fn resolve_libraries_without_link_references_is_empty() {
        let solc_output = solc_output(serde_json::json!({
            "src/Contract.sol": {
                "Contract": {
                    "abi": [],
                    "evm": {"bytecode": {"object": "0x6080"}}
                }
            }
        }));

        let libraries = Linker::resolve_libraries(&solc_output).unwrap();
        assert!(libraries.is_empty());
    }

    #[test]
    fn resolves_linked_libraries() {
        let solc_output = solc_output(serde_json::json!({
            "src/Contract.sol": {
                "Contract": {
                    "abi": [],
                    "evm": {
                        "bytecode": {
                            "object": "0x6080",
                            "linkReferences": {
                                "src/MathLib.sol": {
                                    "MathLib": [{"start": 116, "length": 20}]
                                }
                            }
                        }
                    }
                }
            },
            "src/MathLib.sol": {
                "MathLib": {
                    "abi": [],
                    "evm": {"bytecode": {"object": "0x6081"}}
                }
            }
        }));

        let libraries = Linker::resolve_libraries(&solc_output).unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].id, "src/MathLib.sol:MathLib");
        assert_eq!(libraries[0].initcode, "0x6081");
    }

    #[test]
    fn resolves_nested_linked_libraries() {
        let solc_output = solc_output(serde_json::json!({
            "src/Contract.sol": {
                "Contract": {
                    "abi": [],
                    "evm": {
                        "bytecode": {
                            "object": "0x6080",
                            "linkReferences": {
                                "src/MathLib.sol": {
                                    "MathLib": [{"start": 116, "length": 20}]
                                }
                            }
                        }
                    }
                }
            },
            "src/MathLib.sol": {
                "MathLib": {
                    "abi": [],
                    "evm": {
                        "bytecode": {
                            "object": "0x6081",
                            "linkReferences": {
                                "src/InnerLib.sol": {
                                    "InnerLib": [{"start": 10, "length": 20}]
                                }
                            }
                        }
                    }
                }
            },
            "src/InnerLib.sol": {
                "InnerLib": {
                    "abi": [],
                    "evm": {"bytecode": {"object": "0x6082"}}
                }
            }
        }));

        let libraries = Linker::resolve_libraries(&solc_output).unwrap();
        assert_eq!(libraries.len(), 1);
        let nested = &libraries[0].libraries;
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].id, "src/InnerLib.sol:InnerLib");
        assert_eq!(nested[0].initcode, "0x6082");
    }

    #[test]
    fn missing_library_source_fails() {
        let solc_output = solc_output(serde_json::json!({
            "src/Contract.sol": {
                "Contract": {
                    "abi": [],
                    "evm": {
                        "bytecode": {
                            "object": "0x6080",
                            "linkReferences": {
                                "src/MathLib.sol": {
                                    "MathLib": [{"start": 116, "length": 20}]
                                }
                            }
                        }
                    }
                }
            }
        }));

        let err = Linker::resolve_libraries(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "library source `src/MathLib.sol` not found in compilation output"
        );
    }

    #[test]
    fn missing_library_contract_fails() {
        let solc_output = solc_output(serde_json::json!({
            "src/Contract.sol": {
                "Contract": {
                    "abi": [],
                    "evm": {
                        "bytecode": {
                            "object": "0x6080",
                            "linkReferences": {
                                "src/MathLib.sol": {
                                    "MathLib": [{"start": 116, "length": 20}]
                                }
                            }
                        }
                    }
                }
            },
            "src/MathLib.sol": {
                "OtherLib": {
                    "abi": [],
                    "evm": {"bytecode": {"object": "0x6081"}}
                }
            }
        }));

        let err = Linker::resolve_libraries(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "library `src/MathLib.sol:MathLib` not found in compilation output"
        );
    }

    #[test]
    fn missing_library_initcode_fails() {
        let solc_output = solc_output(serde_json::json!({
            "src/Contract.sol": {
                "Contract": {
                    "abi": [],
                    "evm": {
                        "bytecode": {
                            "object": "0x6080",
                            "linkReferences": {
                                "src/MathLib.sol": {
                                    "MathLib": [{"start": 116, "length": 20}]
                                }
                            }
                        }
                    }
                }
            },
            "src/MathLib.sol": {
                "MathLib": {
                    "abi": [],
                    "evm": {"bytecode": {"object": ""}}
                }
            }
        }));

        let err = Linker::resolve_libraries(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "library `src/MathLib.sol:MathLib` has empty initcode"
        );
    }
}
