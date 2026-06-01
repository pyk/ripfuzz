//! Thread-safe shared failed corpus item for the shrinker.

use std::sync::Arc;

use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::StateMutability;
use anyhow::{Result, ensure};
use parking_lot::RwLock;

use crate::corpus::random::{RandomDynSolValue, random_uint};
use crate::corpus::{Call, CorpusConfig, ExtractedLiterals, Item};

#[derive(Debug)]
struct SharedFailedCorpusItemInner {
    current: RwLock<Item>,
    target_functions: Vec<alloy_json_abi::Function>,
    literals: ExtractedLiterals,
}

/// Thread-safe wrapper around a single failing corpus item that shrinker
/// threads compete to minimize.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct SharedFailedCorpusItem {
    inner: Arc<SharedFailedCorpusItemInner>,
}

impl SharedFailedCorpusItem {
    /// Create a new shared failed corpus item from a seed item and a
    /// [`CorpusConfig`](crate::fuzzer::CorpusConfig).
    pub fn new(item: Item, config: CorpusConfig) -> Self {
        Self {
            inner: Arc::new(SharedFailedCorpusItemInner {
                current: RwLock::new(item),
                target_functions: config.target_functions,
                literals: config.literals,
            }),
        }
    }

    /// Replace the stored item if the new one is smaller (fewer calls).
    pub fn replace_item(&self, item: Item) {
        let mut current = self.inner.current.write();
        if item.calls.len() < current.calls.len() {
            *current = item;
        }
    }

    /// Return a cloned snapshot of the current item.
    pub fn item(&self) -> Item {
        self.inner.current.read().clone()
    }

    /// Return a mutated copy of the current item for the shrinker to try.
    pub fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        let mut item = self.inner.current.read().clone();
        let _ = self.mutate_item(rng, &mut item);
        item
    }

    /// Remove a random call from the item.
    ///
    /// Returns an error if the item contains only a single call.
    pub fn remove_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            item.calls.len() > 1,
            "item must contain at least one call to remove"
        );
        let pos = rng.usize(0..item.calls.len());
        item.calls.remove(pos);
        Ok(())
    }

    /// Apply a randomly selected shrink-oriented mutation to the item.
    ///
    /// Builds a stack-only list of mutations that are legal for the
    /// current item state, picks one uniformly, and executes it. If no
    /// mutation is applicable the item is left unchanged.
    pub fn mutate_item(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let mut ops = [0u8; 4];
        let mut count = 0usize;

        if item.calls.len() > 1 {
            ops[count] = 0;
            count += 1;
            ops[count] = 1;
            count += 1;
        }
        if !item.calls.is_empty() {
            ops[count] = 2;
            count += 1;
            ops[count] = 3;
            count += 1;
        }

        ensure!(count > 0, "no applicable mutations for this item");

        match ops[rng.usize(0..count)] {
            0 => self.remove_call(rng, item),
            1 => self.swap_call(rng, item),
            2 => self.replace_call(rng, item),
            3 => self.update_args(rng, item),
            _ => unreachable!(),
        }
    }

    fn swap_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let len = item.calls.len();
        ensure!(len > 1, "item must contain at least two calls to swap");
        let a = rng.usize(0..len);
        let b = (a + rng.usize(1..len)) % len;
        item.calls.swap(a, b);
        Ok(())
    }

    fn replace_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            !item.calls.is_empty(),
            "item must contain at least one call"
        );
        let pos = rng.usize(0..item.calls.len());
        item.calls[pos] = self.generate_call(rng);
        Ok(())
    }

    fn update_args(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            !item.calls.is_empty(),
            "item must contain at least one call"
        );
        let pos = rng.usize(0..item.calls.len());
        let call = &mut item.calls[pos];
        let values: Vec<DynSolValue> = call
            .function
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| ty.random(rng, &self.inner.literals))
            .collect();
        call.args = DynSolValue::Tuple(values);
        Ok(())
    }

    fn generate_call(&self, rng: &mut fastrand::Rng) -> Call {
        let functions = &self.inner.target_functions;
        if functions.is_empty() {
            return Call::default();
        }
        let idx = rng.usize(0..functions.len());
        let func = &functions[idx];
        let values: Vec<DynSolValue> = func
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| ty.random(rng, &self.inner.literals))
            .collect();
        let value = if func.state_mutability == StateMutability::Payable {
            Some(random_uint(rng, 256, &self.inner.literals))
        } else {
            None
        };

        Call {
            function: func.clone(),
            args: DynSolValue::Tuple(values),
            value,
            ..Default::default()
        }
    }
}
