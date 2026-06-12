//! Context for compiling Apxm modules.

use crate::ffi;
use apxm_core::error::compiler::Result;
use std::marker::PhantomData;

/// A context for compiling Apxm modules.
pub struct Context {
    raw: *mut ffi::ApxmCompilerContext,
    _marker: PhantomData<ffi::ApxmCompilerContext>,
}

impl Context {
    /// Creates a new context.
    pub fn new() -> Result<Self> {
        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_compiler_context_create() },
            "context creation",
        )?;

        Ok(Self {
            raw,
            _marker: PhantomData,
        })
    }

    /// Returns a raw pointer to the context.
    pub fn as_ptr(&self) -> *mut ffi::ApxmCompilerContext {
        self.raw
    }
}

impl Drop for Context {
    /// Destroys the context.
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                ffi::apxm_compiler_context_destroy(self.raw);
            }
        }
    }
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

