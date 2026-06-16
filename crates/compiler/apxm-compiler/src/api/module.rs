//! Module API for the Apxm compiler.
//!
//! Modules are the building blocks of Apxm programs.
//! They contain a set of functions, variables, and types available to other modules.
//!
//! Modules can be imported using the `import` keyword, which allows the functions, variables,
//! and types defined in the imported module to be used in the current module.

use crate::api::Context;
use crate::codegen::artifact::parse_wire_dags;
use crate::ffi;
use apxm_artifact::{Artifact, ArtifactMetadata};
use apxm_core::error::Error;
use apxm_core::error::codes::ErrorCode;
use apxm_core::error::compiler::{CompilerError, Result};
use std::ffi::CString;
use std::marker::PhantomData;
use std::os::raw::c_char;
use std::ptr;

pub(crate) fn invalid_input_error(message: impl Into<String>) -> CompilerError {
    CompilerError::InvalidInput(Box::new(Error::new_generic(
        ErrorCode::InternalError,
        message,
    )))
}

pub struct Module {
    raw: *mut ffi::ApxmModule,
    _context: PhantomData<Context>,
}

impl Module {
    /// # Safety
    ///
    /// The caller must ensure that `raw` is a valid pointer to an `ApxmModule`
    /// and that ownership is properly transferred to the returned `Module`.
    pub unsafe fn from_raw(raw: *mut ffi::ApxmModule) -> Self {
        Self {
            raw,
            _context: PhantomData,
        }
    }

    /// Parses the given source string into a module.
    pub fn parse(context: &Context, source: &str) -> Result<Self> {
        let c_source = CString::new(source)
            .map_err(|e| invalid_input_error(format!("Invalid source string: {}", e)))?;

        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_module_parse(context.as_ptr(), c_source.as_ptr()) },
            "module parsing",
        )?;

        Ok(unsafe { Self::from_raw(raw) })
    }

    /// Parses the given source file into a module.
    pub fn parse_file(context: &Context, path: &std::path::Path) -> Result<Self> {
        let source = std::fs::read_to_string(path).map_err(CompilerError::Io)?;
        Self::parse(context, &source)
    }

    /// Verifies the module's structure and syntax.
    pub fn verify(&self) -> Result<()> {
        ffi::handle_bool_result(
            unsafe { ffi::apxm_module_verify(self.raw) },
            "module verification",
        )
    }

    pub fn to_string(&self) -> Result<String> {
        let c_str = ffi::handle_null_result(
            unsafe { ffi::apxm_module_to_string(self.raw) },
            "module serialization",
        )?;

        let result = unsafe {
            std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned()
        };

        unsafe { ffi::apxm_string_free(c_str as *mut _) };

        Ok(result)
    }

    pub fn as_ptr(&self) -> *mut ffi::ApxmModule {
        self.raw
    }

    pub fn generate_artifact(&self) -> Result<Artifact> {
        self.generate_artifact_with_name(None)
    }

    pub fn generate_artifact_with_name(&self, module_name: Option<&str>) -> Result<Artifact> {
        self.generate_artifact_with_manifest(module_name, None)
    }

    pub fn generate_artifact_with_manifest(
        &self,
        module_name: Option<&str>,
        manifest: Option<&[crate::passes::PythonToolManifestEntry]>,
    ) -> Result<Artifact> {
        self.generate_artifact_with_manifest_and_caps(
            module_name,
            manifest,
            &std::collections::HashSet::new(),
        )
    }

    /// Like [`generate_artifact_with_manifest`], but `known_caps` declares
    /// capabilities the host already has registered at runtime (provider/pack
    /// blocks). They satisfy the tool-binding check (E712) alongside builtins and
    /// in-graph `REGISTER_CAPABILITY` nodes — a standalone compile passes an empty
    /// set and stays strict.
    pub fn generate_artifact_with_manifest_and_caps(
        &self,
        module_name: Option<&str>,
        manifest: Option<&[crate::passes::PythonToolManifestEntry]>,
        known_caps: &std::collections::HashSet<String>,
    ) -> Result<Artifact> {
        let payload = self.emit_artifact_payload(module_name)?;
        let mut dags = parse_wire_dags(&payload)?;
        crate::artifact_validation::validate_template_placeholders(&dags)
            .map_err(invalid_input_error)?;
        crate::token_estimate::refine_token_estimates(&mut dags);

        // Post-MLIR tool-binding-check: validate INV_TOOL ↔ REGISTER_CAPABILITY
        // and copy `python_handler_id` from registrations onto invocations.
        // W721/W723 warnings are logged here (non-fatal).
        for dag in dags.iter_mut() {
            let warnings = crate::passes::tool_binding_check_dag(dag, manifest, known_caps)?;
            for w in &warnings {
                eprintln!("warning[{}]: {}", w.code, w.message);
            }
            crate::passes::bind_python_handlers_to_dag(dag)?;
        }

        // Find entry DAG for metadata naming
        let entry_dag = dags.iter().find(|d| d.metadata.is_entry);
        let name = entry_dag
            .and_then(|d| d.metadata.name.clone())
            .or_else(|| dags.first().and_then(|d| d.metadata.name.clone()));

        let metadata = ArtifactMetadata::new(name, crate::VERSION);
        Ok(Artifact::new(metadata, dags))
    }

    pub fn generate_artifact_bytes(&self) -> Result<Vec<u8>> {
        self.generate_artifact_bytes_with_name(None)
    }

    /// Artifact bytes with host-declared `known_caps` for the tool-binding check.
    pub fn generate_artifact_bytes_with_known_caps(
        &self,
        known_caps: &std::collections::HashSet<String>,
    ) -> Result<Vec<u8>> {
        self.generate_artifact_with_manifest_and_caps(None, None, known_caps)?
            .to_bytes()
            .map_err(|err| invalid_input_error(err.to_string()))
    }

    pub fn generate_artifact_bytes_with_name(&self, module_name: Option<&str>) -> Result<Vec<u8>> {
        self.generate_artifact_with_name(module_name)?
            .to_bytes()
            .map_err(|err| invalid_input_error(err.to_string()))
    }

    pub fn generate_artifact_to_path<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let bytes = self.generate_artifact_bytes()?;
        std::fs::write(path, &bytes).map_err(CompilerError::Io)
    }

    fn emit_artifact_payload(&self, module_name: Option<&str>) -> Result<Vec<u8>> {
        let module_name_cstr = module_name
            .map(|name| {
                CString::new(name)
                    .map_err(|e| invalid_input_error(format!("Invalid module name: {e}")))
            })
            .transpose()?;

        let c_options = ffi::ApxmArtifactOptions {
            module_name: module_name_cstr
                .as_ref()
                .map(|c| c.as_ptr())
                .unwrap_or(ptr::null()),
            emit_debug_json: false,
            target_version: ptr::null(),
        };

        let raw = ffi::handle_null_result(
            unsafe { ffi::apxm_codegen_emit_artifact(self.raw, &c_options) },
            "artifact generation",
        )?;

        unsafe { copy_artifact_buffer(raw) }
    }
}

unsafe fn copy_artifact_buffer(ptr: *mut c_char) -> Result<Vec<u8>> {
    struct BufferGuard(*mut c_char);

    impl Drop for BufferGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { ffi::apxm_codegen_free(self.0) };
            }
        }
    }

    let guard = BufferGuard(ptr);
    if guard.0.is_null() {
        return Err(invalid_input_error("Artifact emitter returned null buffer"));
    }

    let mut len_buf = [0u8; 8];
    unsafe {
        len_buf.copy_from_slice(std::slice::from_raw_parts(guard.0 as *const u8, 8));
    }
    let len = u64::from_le_bytes(len_buf) as usize;

    let data = unsafe {
        let data_ptr = guard.0.add(std::mem::size_of::<u64>()) as *const u8;
        std::slice::from_raw_parts(data_ptr, len)
    };
    Ok(data.to_vec())
}

impl Drop for Module {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                ffi::apxm_module_destroy(self.raw);
            }
        }
    }
}
