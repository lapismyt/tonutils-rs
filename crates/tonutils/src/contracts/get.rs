//! Shared get-method execution and stack decoding helpers.

use std::vec::IntoIter;

use crate::{
    provider::Provider,
    tvm::{Address, TvmStack, TvmStackEntry},
};

use super::{
    provider::{Contract, ContractError},
    stack::{FromTvmStackEntry, ToTvmStack, TvmStackConversionError},
};

/// Runs a named get-method and returns its raw result entries.
pub async fn run_get_method<P, A>(
    provider: &mut P,
    address: Address,
    method: &str,
    args: A,
) -> Result<Vec<TvmStackEntry>, ContractError>
where
    P: Provider + ?Sized,
    A: ToTvmStack,
{
    let stack: TvmStack = args
        .to_tvm_stack()
        .map_err(ContractError::StackConversion)?;
    Contract::new(provider, address)
        .run_get_method_by_name_typed(None, method, stack)
        .await
}

/// Decodes the next stack entry.
pub fn next_entry<T>(entries: &mut IntoIter<TvmStackEntry>) -> Result<T, ContractError>
where
    T: FromTvmStackEntry,
{
    let entry = entries.next().ok_or(ContractError::StackConversion(
        TvmStackConversionError::StackArityMismatch {
            expected: 1,
            actual: 0,
        },
    ))?;
    T::from_tvm_stack_entry(entry).map_err(ContractError::StackConversion)
}

/// Rejects trailing stack entries.
pub fn ensure_consumed(entries: &IntoIter<TvmStackEntry>) -> Result<(), ContractError> {
    if entries.len() == 0 {
        Ok(())
    } else {
        Err(ContractError::StackConversion(
            TvmStackConversionError::StackArityMismatch {
                expected: 0,
                actual: entries.len(),
            },
        ))
    }
}
