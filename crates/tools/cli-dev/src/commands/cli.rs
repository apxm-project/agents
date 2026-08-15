//! CLI type definitions for `apxm-dev`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "apxm-dev")]
#[command(about = "Dekk-owned compiler and runtime conformance binary", long_about = None)]
pub struct DevCli {
    /// Optional config path
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: DevCommands,
}

#[derive(Subcommand)]
pub enum DevCommands {
    /// Compile a package to canonical AIR JSON (conformance fixture).
    CompileServiceCanonical {
        /// Agent directory
        agent_dir: PathBuf,
    },
    /// Execute canonical AIR through the Runtime Service fixture path.
    ExecuteCanonical {
        /// Canonical AIR JSON file.
        input: PathBuf,
        /// Invocation Admission JSON.
        #[arg(long, value_name = "PATH")]
        invocation_admission: PathBuf,
        /// Release bytes.
        #[arg(long, value_name = "PATH")]
        release: PathBuf,
        /// Provenance bytes.
        #[arg(long, value_name = "PATH")]
        provenance: PathBuf,
        /// Optional package root for handlers/skills.
        #[arg(long, value_name = "DIR")]
        package: Option<PathBuf>,
    },
    /// Generate frontend assets from Rust-owned registries.
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
    /// Lower a FrontendGraph document to canonical AIR.
    CanonicalAir {
        /// Canonical frontend graph JSON file. Omit to read stdin.
        input: Option<PathBuf>,
    },
    /// Validate canonical AIR source against the AIS contract.
    Validate {
        /// Input workflow source (.air)
        input: PathBuf,
        /// Skip Tier 2 environment checks.
        #[arg(long)]
        no_check_resources: bool,
    },
    /// Analyze an AIR workflow for parallelism, critical path, and execution phases.
    Analyze {
        /// Input workflow source (.air)
        input: PathBuf,
    },
    /// Browse AIS operations (the agent instruction set).
    Ops {
        #[command(subcommand)]
        action: OpsAction,
    },
    /// Browse workflow templates (starter patterns).
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Explain what a workflow does OR explain an error code.
    Explain {
        /// Error code (e.g., E511) or path to workflow source (.air)
        target: String,
    },
}

#[derive(Subcommand)]
pub enum TemplateAction {
    /// List available workflow templates
    List,
    /// Show a specific template
    Show {
        /// Template name (e.g., ask, pipeline, fan-out, map-reduce)
        name: String,
    },
}

#[derive(Subcommand)]
pub enum CodegenAction {
    /// Generate the Python frontend bindings into apxm_program/_generated
    Frontend {
        /// Check that the generated Python files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript types into the TypeScript authoring frontend
    Typescript {
        /// Output file path for generated TypeScript
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
        #[arg(long)]
        check: bool,
    },
    /// Generate @apxm/frontend TypeScript metadata into src/generated
    TypescriptFrontend {
        /// Output directory for generated TypeScript frontend files
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Check that the generated files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript event kind constants into the Studio frontend
    EventKinds {
        /// Output file path for generated TypeScript event kinds
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
        #[arg(long)]
        check: bool,
    },
    /// Generate the capability id catalogue into both authoring frontends
    Capabilities {
        /// Check that the generated catalogue files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the permission decision vocabulary into both authoring frontends
    Permissions {
        /// Check that the generated decision files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the source graph vocabulary into both authoring frontends
    FrontendVocabulary {
        /// Check that the generated vocabulary files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the FrontendGraph contract record types into both authoring
    /// frontends
    FrontendRecords {
        /// Check that the generated record files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the FrontendGraph contract record serializers into both
    /// authoring frontends
    FrontendSerializers {
        /// Check that the generated serializer files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the shared frontend-conformance harness into every authoring
    /// frontend and its test runner
    FrontendConformance {
        /// Check that the generated harness files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the authoring diagnostic codes into both authoring frontends
    Diagnostics {
        /// Check that the generated diagnostic files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the reference tables the authoring documentation carries
    Docs {
        /// Check that the generated documentation regions are up to date
        /// without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the op-spec AIS operation catalog + vectors fixture
    OpSpec {
        /// Output directory for the generated catalog + vectors files
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Check that the generated files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
pub enum OpsAction {
    /// List all AIS operations
    List {
        /// Filter by category (e.g., reasoning, memory, tools, control_flow)
        #[arg(long)]
        category: Option<String>,
    },
    /// Show detailed info for a specific operation
    Show {
        /// Operation name (e.g., ASK, THINK, INV, FLOW_CALL)
        name: String,
    },
    /// Show accumulated op-usage counts from real executions
    Usage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_service_canonical_accepts_explicit_package_path() {
        let cli = DevCli::try_parse_from(["apxm-dev", "compile-service-canonical", "/tmp/agent"])
            .expect("compile-service-canonical parses");

        match cli.command {
            DevCommands::CompileServiceCanonical { agent_dir } => {
                assert_eq!(agent_dir, PathBuf::from("/tmp/agent"));
            }
            _ => panic!("expected compile-service-canonical command"),
        }
    }

    #[test]
    fn compile_service_canonical_rejects_missing_package_path() {
        let err = match DevCli::try_parse_from(["apxm-dev", "compile-service-canonical"]) {
            Ok(_) => panic!("compile-service-canonical requires an agent package path"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn execute_canonical_accepts_exact_admission_inputs() {
        let cli = DevCli::try_parse_from([
            "apxm-dev",
            "execute-canonical",
            "/tmp/program.air",
            "--invocation-admission",
            "/tmp/admission.json",
            "--release",
            "/tmp/release.json",
            "--provenance",
            "/tmp/provenance.json",
        ])
        .expect("execute-canonical parses");

        match cli.command {
            DevCommands::ExecuteCanonical {
                input,
                invocation_admission,
                release,
                provenance,
                package,
            } => {
                assert_eq!(input, PathBuf::from("/tmp/program.air"));
                assert_eq!(invocation_admission, PathBuf::from("/tmp/admission.json"));
                assert_eq!(release, PathBuf::from("/tmp/release.json"));
                assert_eq!(provenance, PathBuf::from("/tmp/provenance.json"));
                assert_eq!(package, None);
            }
            _ => panic!("expected execute-canonical command"),
        }
    }

    #[test]
    fn execute_canonical_rejects_missing_invocation_admission() {
        let error =
            match DevCli::try_parse_from(["apxm-dev", "execute-canonical", "/tmp/program.air"]) {
                Ok(_) => panic!("execute-canonical must require exact host authority"),
                Err(error) => error,
            };
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}
