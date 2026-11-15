//! Optimized single-call get-method emulation.
//!
//! Upstream exposes `tvm_emulator_emulate_run_method`, which accepts the code,
//! data, stack, `c7`, libraries, and method id in one BoC request instead of
//! allocating a `TvmEmulator` object for every call.

use std::ffi::{CStr, c_char, c_void};
use std::slice;
use std::sync::Arc;

use crate::tvm::{Builder, Cell, Slice, VmStack, deserialize_boc};

use super::{EmulatorEngine, EmulatorError};

/// Parameters for [`EmulatorEngine::emulate_run_method`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunMethodParams {
    /// Smart-contract code.
    pub code: Arc<Cell>,
    /// Smart-contract data.
    pub data: Arc<Cell>,
    /// Get-method input stack.
    pub stack: VmStack,
    /// `c7` stack. Upstream expects a single tuple value at the top.
    pub c7: VmStack,
    /// Shared-library dictionary, when the contract uses libraries.
    pub libraries: Option<Arc<Cell>>,
    /// Numeric get-method id.
    pub method_id: u32,
}

impl RunMethodParams {
    /// Creates parameters without a library dictionary.
    pub fn new(
        code: Arc<Cell>,
        data: Arc<Cell>,
        stack: VmStack,
        c7: VmStack,
        method_id: u32,
    ) -> Self {
        Self {
            code,
            data,
            stack,
            c7,
            libraries: None,
            method_id,
        }
    }

    /// Sets the shared-library dictionary.
    #[must_use]
    pub fn libraries(mut self, libraries: Arc<Cell>) -> Self {
        self.libraries = Some(libraries);
        self
    }

    fn to_cell(&self) -> Result<Arc<Cell>, EmulatorError> {
        let libraries = match &self.libraries {
            Some(libraries) => libraries.clone(),
            None => empty_dictionary()?,
        };

        let mut params = Builder::new();
        params.store_ref(self.c7.to_cell()?)?;
        params.store_ref(libraries)?;
        let params = params.build()?;

        let mut root = Builder::new();
        root.store_ref(self.code.clone())?;
        root.store_ref(self.data.clone())?;
        root.store_ref(self.stack.to_cell()?)?;
        root.store_ref(params)?;
        root.store_u32(self.method_id)?;
        Ok(root.build()?)
    }
}

/// Result of an optimized get-method call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmulatedRunMethodResult {
    /// TVM exit code.
    pub exit_code: i32,
    /// Gas consumed by the method.
    pub gas_used: u64,
    /// Result stack.
    pub stack: VmStack,
}

/// Result of an optimized get-method call that also returns the VM log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedRunMethodResult {
    /// Decoded emulation result.
    pub result: EmulatedRunMethodResult,
    /// VM execution log.
    pub vm_log: String,
}

#[repr(C)]
struct RawDetailedResult {
    response: *const c_char,
    log: *const c_char,
}

impl EmulatorEngine {
    /// Runs a get-method with the optimized request/response ABI.
    ///
    /// This does not require a [`super::TvmEmulator`] object and is suitable
    /// for one-off calls. The request is serialized according to the upstream
    /// `code:^Cell data:^Cell stack:^VmStack params:^[c7:^VmStack libs:^Cell]
    /// method_id:(## 32)` scheme.
    ///
    /// # Errors
    ///
    /// Propagates request serialization errors and returns
    /// [`EmulatorError::InvalidResponse`] when the library returns malformed
    /// data.
    pub fn emulate_run_method(
        &self,
        params: &RunMethodParams,
        gas_limit: i64,
    ) -> Result<EmulatedRunMethodResult, EmulatorError> {
        let encoded = params.to_cell()?;
        let boc = crate::tvm::serialize_boc(&encoded, false)?;
        let length = u32::try_from(boc.len()).map_err(|_| {
            EmulatorError::InvalidInput("run-method request exceeds u32 length".to_owned())
        })?;

        // SAFETY: `boc` remains alive for the duration of the call, `length`
        // matches its size, and the returned pointer is either null or a
        // library-owned length-prefixed buffer.
        let pointer = unsafe {
            (self.ffi.tvm_emulator_emulate_run_method)(
                length,
                boc.as_ptr().cast::<c_char>(),
                gas_limit,
            )
        };
        let bytes = self.copy_length_prefixed(pointer)?;
        parse_result_boc(&bytes)
    }

    /// Runs a get-method with the optimized ABI and returns the VM log.
    ///
    /// # Errors
    ///
    /// Propagates request serialization errors and returns
    /// [`EmulatorError::InvalidResponse`] when the library returns malformed
    /// data.
    pub fn emulate_run_method_detailed(
        &self,
        params: &RunMethodParams,
        gas_limit: i64,
    ) -> Result<DetailedRunMethodResult, EmulatorError> {
        let encoded = params.to_cell()?;
        let boc = crate::tvm::serialize_boc(&encoded, false)?;
        let length = u32::try_from(boc.len()).map_err(|_| {
            EmulatorError::InvalidInput("run-method request exceeds u32 length".to_owned())
        })?;

        // SAFETY: `boc` remains alive for the duration of the call, `length`
        // matches its size, and the returned pointer is either null or an
        // upstream-allocated detailed-result struct.
        let pointer = unsafe {
            (self.ffi.tvm_emulator_emulate_run_method_detailed)(
                length,
                boc.as_ptr().cast::<c_char>(),
                gas_limit,
            )
        };
        if pointer.is_null() {
            return Err(EmulatorError::InvalidResponse(
                "optimized run-method detailed call returned null".to_owned(),
            ));
        }

        let raw = pointer.cast::<RawDetailedResult>();
        // SAFETY: the library returns a valid `RawDetailedResult` pointer for
        // the duration of this call.
        let (response, log) = unsafe { ((*raw).response, (*raw).log) };
        let copied = read_length_prefixed(response).and_then(|bytes| parse_result_boc(&bytes));
        let vm_log = if log.is_null() {
            String::new()
        } else {
            // SAFETY: `log` is a NUL-terminated string owned by the detailed
            // result and stays alive until `run_method_detailed_result_destroy`.
            unsafe { CStr::from_ptr(log) }
                .to_string_lossy()
                .into_owned()
        };

        // SAFETY: `pointer` came from the detailed call and is released
        // exactly once; its response and log fields are freed with it.
        unsafe { (self.ffi.run_method_detailed_result_destroy)(pointer.cast::<c_void>()) };

        Ok(DetailedRunMethodResult {
            result: copied?,
            vm_log,
        })
    }

    fn copy_length_prefixed(&self, pointer: *const c_char) -> Result<Vec<u8>, EmulatorError> {
        let bytes = read_length_prefixed(pointer)?;
        // SAFETY: the returned buffer is allocated by the library and released
        // by `string_destroy`, which is the matching deallocator for this ABI.
        unsafe { (self.ffi.string_destroy)(pointer) };
        Ok(bytes)
    }
}

fn read_length_prefixed(pointer: *const c_char) -> Result<Vec<u8>, EmulatorError> {
    if pointer.is_null() {
        return Err(EmulatorError::InvalidResponse(
            "optimized run-method call returned null".to_owned(),
        ));
    }

    // SAFETY: the ABI prefixes the payload with a native-endian 32-bit length
    // and keeps at least four bytes available at `pointer`.
    let length = unsafe { std::ptr::read_unaligned(pointer.cast::<u32>()) } as usize;
    // SAFETY: the library guarantees `length` bytes follow the prefix.
    Ok(unsafe { slice::from_raw_parts(pointer.add(4).cast::<u8>(), length) }.to_vec())
}

fn empty_dictionary() -> Result<Arc<Cell>, EmulatorError> {
    let mut builder = Builder::new();
    builder.store_bit(false)?;
    Ok(builder.build()?)
}

fn parse_result_boc(bytes: &[u8]) -> Result<EmulatedRunMethodResult, EmulatorError> {
    let cell = deserialize_boc(bytes)?;
    let mut slice = Slice::new(cell);
    let exit_code = i32::from_be_bytes(slice.load_u32()?.to_be_bytes());
    let gas_used = slice.load_u64()?;
    let stack = VmStack::from_cell(slice.load_reference()?)?;
    Ok(EmulatedRunMethodResult {
        exit_code,
        gas_used,
        stack,
    })
}

#[cfg(test)]
mod tests {
    use super::{RunMethodParams, empty_dictionary, parse_result_boc};
    use crate::tvm::{Builder, Cell, Slice, VmStack, VmStackValue};

    fn cell(byte: u8) -> std::sync::Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(byte).unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn request_matches_upstream_reference_layout() {
        let c7 = VmStack::new(vec![VmStackValue::Tuple(Vec::new())]);
        let params = RunMethodParams::new(
            cell(0x11),
            cell(0x22),
            VmStack::new(vec![VmStackValue::int(7)]),
            c7,
            42,
        );

        let root = params.to_cell().unwrap();
        assert_eq!(root.references().len(), 4);
        assert_eq!(root.bit_len(), 32);
        assert_eq!(Slice::new(root.clone()).load_u32().unwrap(), 42);

        let params_cell = &root.references()[3];
        assert_eq!(params_cell.references().len(), 2);
        assert_eq!(params_cell.bit_len(), 0);
    }

    #[test]
    fn parses_length_prefixed_result_payload() {
        let mut builder = Builder::new();
        builder.store_u32(0).unwrap();
        builder.store_u64(123).unwrap();
        builder
            .store_ref(VmStack::default().to_cell().unwrap())
            .unwrap();
        let cell = builder.build().unwrap();

        let result = parse_result_boc(&crate::tvm::serialize_boc(&cell, false).unwrap()).unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.gas_used, 123);
        assert!(result.stack.is_empty());
    }

    #[test]
    fn empty_library_dictionary_encodes_hashmap_e_absent_bit() {
        let dictionary = empty_dictionary().unwrap();
        assert_eq!(dictionary.bit_len(), 1);
        assert_eq!(dictionary.references().len(), 0);
        assert!(!Slice::new(dictionary).load_bit().unwrap());
    }
}
