//! Error codes for all APXM components.
//!
//! Error codes are organized by component and provide a stable identifier
//! for each error type. This enables:
//! - Documentation lookup
//! - Error categorization
//! - Automated fixes
//! - Error tracking/metrics
//!
//! Format: E<component><number>
//! - E001-E099: Parser errors
//! - E101-E199: Type errors
//! - E201-E299: MLIR/Verification errors
//! - E301-E399: Optimization errors
//! - E401-E499: Runtime errors
//! - E900-E999: Generic errors

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error code prefix indicates component
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum ErrorCode {
    // ========================================================================
    // Parser Errors (E001-E099)
    // ========================================================================
    /// E001: Unexpected token
    UnexpectedToken = 1,

    /// E002: Expected expression
    ExpectedExpression = 2,

    /// E003: Expected identifier
    ExpectedIdentifier = 3,

    /// E004: Invalid number literal
    InvalidNumberLiteral = 4,

    /// E005: Invalid string literal
    InvalidStringLiteral = 5,

    /// E006: Unknown keyword
    UnknownKeyword = 6,

    /// E007: Missing closing brace
    MissingClosingBrace = 7,

    /// E008: Missing closing parenthesis
    MissingClosingParen = 8,

    /// E009: Missing closing bracket
    MissingClosingBracket = 9,

    /// E010: Duplicate declaration
    DuplicateDeclaration = 10,

    /// E011: Expected type annotation
    ExpectedTypeAnnotation = 11,

    /// E012: Invalid memory tier
    InvalidMemoryTier = 12,

    /// E013: Expected event type
    ExpectedEventType = 13,

    /// E014: Expected capability name
    ExpectedCapabilityName = 14,

    /// E015: Expected flow name
    ExpectedFlowName = 15,

    /// E016: Expected belief name
    ExpectedBeliefName = 16,

    /// E017: Expected goal name
    ExpectedGoalName = 17,

    /// E018: Expected agent name
    ExpectedAgentName = 18,

    /// E019: Expected memory name
    ExpectedMemoryName = 19,

    /// E020: Invalid operator
    InvalidOperator = 20,

    /// E021: Expected function name
    ExpectedFunctionName = 21,

    /// E022: Expected member name
    ExpectedMemberName = 22,

    /// E023: Expected array index
    ExpectedArrayIndex = 23,

    /// E024: Expected parameter
    ExpectedParameter = 24,

    /// E025: Expected return value
    ExpectedReturnValue = 25,

    /// E026: Expected condition
    ExpectedCondition = 26,

    /// E027: Expected loop variable
    ExpectedLoopVariable = 27,

    /// E028: Expected collection
    ExpectedCollection = 28,

    /// E029: Expected code string
    ExpectedCodeString = 29,

    /// E030: Expected trace ID
    ExpectedTraceId = 30,

    /// E031: Expected goal string
    ExpectedGoalString = 31,

    /// E032: Expected template string
    ExpectedTemplateString = 32,

    /// E033: Expected recipient
    ExpectedRecipient = 33,

    /// E034: Syntax error
    SyntaxError = 34,

    // ========================================================================
    // Type Errors (E101-E199)
    // ========================================================================
    /// E101: Type mismatch
    TypeMismatch = 101,

    /// E102: Undefined variable
    UndefinedVariable = 102,

    /// E103: Invalid type annotation
    InvalidTypeAnnotation = 103,

    /// E104: Type inference failed
    TypeInferenceFailed = 104,

    /// E105: Type not found
    TypeNotFound = 105,

    /// E106: Invalid type conversion
    InvalidTypeConversion = 106,

    /// E107: Type annotation required
    TypeAnnotationRequired = 107,

    // ========================================================================
    // MLIR/Verification Errors (E201-E299)
    // ========================================================================
    /// E201: MLIR verification failed
    MLIRVerificationFailed = 201,

    /// E202: Invalid operation
    InvalidOperation = 202,

    /// E203: DAG cycle detected
    DagCycleDetected = 203,

    /// E204: Missing required operand
    MissingRequiredOperand = 204,

    /// E205: Invalid operand type
    InvalidOperandType = 205,

    /// E206: Operation not found
    OperationNotFound = 206,

    /// E207: Invalid operation result
    InvalidOperationResult = 207,

    // ========================================================================
    // Optimization Errors (E301-E399)
    // ========================================================================
    /// E301: Pass execution failed
    PassExecutionFailed = 301,

    /// E302: Optimization conflict
    OptimizationConflict = 302,

    /// E303: Pass not found
    PassNotFound = 303,

    /// E304: Pass dependency failed
    PassDependencyFailed = 304,

    // ========================================================================
    // Runtime Errors (E401-E499)
    // ========================================================================
    /// E401: Scheduler error
    SchedulerError = 401,

    /// E402: Operation execution failed
    OperationExecutionFailed = 402,

    /// E403: Timeout
    Timeout = 403,

    /// E404: Capability not found
    CapabilityNotFound = 404,

    /// E405: Memory access error
    MemoryAccessError = 405,

    /// E406: LLM backend error
    LLMBackendError = 406,

    // ========================================================================
    // Semantic Validation Errors (E501-E599)
    // ========================================================================
    /// E501: Template placeholder out of bounds
    TemplatePlaceholderBounds = 501,
    /// E502: COMMUNICATE recipient not spawned in graph
    CommunicateRecipientNotSpawned = 502,
    /// E503: Duplicate agent name in SPAWN_AGENT nodes
    DuplicateAgentName = 503,
    /// E504: INV params_json placeholder out of bounds
    InvPlaceholderBounds = 504,
    /// E505: Parameter count vs entry node count mismatch
    ParameterArityMismatch = 505,
    /// E506: Agent profile not registered
    ProfileNotRegistered = 506,
    /// E507: INV(acp) references unregistered agent
    InvAcpAgentNotRegistered = 507,
    /// E508: Backend not registered
    BackendNotRegistered = 508,
    /// E509: Model not found in any backend
    ModelNotFound = 509,
    /// E510: Capability not registered
    CapabilityNotRegistered = 510,
    /// E512: Dead node detected (output never used)
    DeadNode = 512,
    /// E513: Missing return value (no exit node)
    MissingReturnValue = 513,
    /// E514: COMMUNICATE node runs before its SPAWN_AGENT
    CommunicateBeforeSpawn = 514,
    /// E515: Circular model_profile reference
    CircularModelProfile = 515,
    /// E516: Empty template string
    EmptyTemplate = 516,
    /// E517: Unchecked memory read (QMEM result used without fallback)
    UncheckedMemoryRead = 517,
    /// E518: const_str with dynamic input (should only take literals)
    ConstStrWithDynamicInput = 518,

    // ========================================================================
    // Capability Binding Errors (E701-E799)
    // ========================================================================
    /// E712: a `capability.invoke` reference resolves to no admitted capability
    UnboundCapability = 712,
    /// E721: a declared capability whose name is never invoked
    UnusedCapability = 721,
    /// E722: schema-vs-signature drift on a declared capability binding
    SchemaDrift = 722,

    // ========================================================================
    // Generic Errors (E900-E999)
    // ========================================================================
    /// E900: Internal error
    InternalError = 900,

    /// E901: Not implemented
    NotImplemented = 901,

    /// E902: Invalid configuration
    InvalidConfiguration = 902,

    /// E903: IO error
    IoError = 903,
}

impl ErrorCode {
    /// Get error code as string (e.g., "E001")
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::UnexpectedToken => "E001",
            ErrorCode::ExpectedExpression => "E002",
            ErrorCode::ExpectedIdentifier => "E003",
            ErrorCode::InvalidNumberLiteral => "E004",
            ErrorCode::InvalidStringLiteral => "E005",
            ErrorCode::UnknownKeyword => "E006",
            ErrorCode::MissingClosingBrace => "E007",
            ErrorCode::MissingClosingParen => "E008",
            ErrorCode::MissingClosingBracket => "E009",
            ErrorCode::DuplicateDeclaration => "E010",
            ErrorCode::ExpectedTypeAnnotation => "E011",
            ErrorCode::InvalidMemoryTier => "E012",
            ErrorCode::ExpectedEventType => "E013",
            ErrorCode::ExpectedCapabilityName => "E014",
            ErrorCode::ExpectedFlowName => "E015",
            ErrorCode::ExpectedBeliefName => "E016",
            ErrorCode::ExpectedGoalName => "E017",
            ErrorCode::ExpectedAgentName => "E018",
            ErrorCode::ExpectedMemoryName => "E019",
            ErrorCode::InvalidOperator => "E020",
            ErrorCode::ExpectedFunctionName => "E021",
            ErrorCode::ExpectedMemberName => "E022",
            ErrorCode::ExpectedArrayIndex => "E023",
            ErrorCode::ExpectedParameter => "E024",
            ErrorCode::ExpectedReturnValue => "E025",
            ErrorCode::ExpectedCondition => "E026",
            ErrorCode::ExpectedLoopVariable => "E027",
            ErrorCode::ExpectedCollection => "E028",
            ErrorCode::ExpectedCodeString => "E029",
            ErrorCode::ExpectedTraceId => "E030",
            ErrorCode::ExpectedGoalString => "E031",
            ErrorCode::ExpectedTemplateString => "E032",
            ErrorCode::ExpectedRecipient => "E033",
            ErrorCode::SyntaxError => "E034",
            ErrorCode::TypeMismatch => "E101",
            ErrorCode::UndefinedVariable => "E102",
            ErrorCode::InvalidTypeAnnotation => "E103",
            ErrorCode::TypeInferenceFailed => "E104",
            ErrorCode::TypeNotFound => "E105",
            ErrorCode::InvalidTypeConversion => "E106",
            ErrorCode::TypeAnnotationRequired => "E107",
            ErrorCode::MLIRVerificationFailed => "E201",
            ErrorCode::InvalidOperation => "E202",
            ErrorCode::DagCycleDetected => "E203",
            ErrorCode::MissingRequiredOperand => "E204",
            ErrorCode::InvalidOperandType => "E205",
            ErrorCode::OperationNotFound => "E206",
            ErrorCode::InvalidOperationResult => "E207",
            ErrorCode::PassExecutionFailed => "E301",
            ErrorCode::OptimizationConflict => "E302",
            ErrorCode::PassNotFound => "E303",
            ErrorCode::PassDependencyFailed => "E304",
            ErrorCode::SchedulerError => "E401",
            ErrorCode::OperationExecutionFailed => "E402",
            ErrorCode::Timeout => "E403",
            ErrorCode::CapabilityNotFound => "E404",
            ErrorCode::MemoryAccessError => "E405",
            ErrorCode::LLMBackendError => "E406",
            ErrorCode::TemplatePlaceholderBounds => "E501",
            ErrorCode::CommunicateRecipientNotSpawned => "E502",
            ErrorCode::DuplicateAgentName => "E503",
            ErrorCode::InvPlaceholderBounds => "E504",
            ErrorCode::ParameterArityMismatch => "E505",
            ErrorCode::ProfileNotRegistered => "E506",
            ErrorCode::InvAcpAgentNotRegistered => "E507",
            ErrorCode::BackendNotRegistered => "E508",
            ErrorCode::ModelNotFound => "E509",
            ErrorCode::CapabilityNotRegistered => "E510",
            ErrorCode::DeadNode => "E512",
            ErrorCode::MissingReturnValue => "E513",
            ErrorCode::CommunicateBeforeSpawn => "E514",
            ErrorCode::CircularModelProfile => "E515",
            ErrorCode::EmptyTemplate => "E516",
            ErrorCode::UncheckedMemoryRead => "E517",
            ErrorCode::ConstStrWithDynamicInput => "E518",
            ErrorCode::UnboundCapability => "E712",
            ErrorCode::UnusedCapability => "E721",
            ErrorCode::SchemaDrift => "E722",
            ErrorCode::InternalError => "E900",
            ErrorCode::NotImplemented => "E901",
            ErrorCode::InvalidConfiguration => "E902",
            ErrorCode::IoError => "E903",
        }
    }

    /// Get component name
    pub fn component(&self) -> &'static str {
        let code = *self as u32;
        if code < 100 {
            "parser"
        } else if code < 200 {
            "type"
        } else if code < 300 {
            "mlir"
        } else if code < 400 {
            "optimization"
        } else if code < 500 {
            "runtime"
        } else if code < 600 {
            "semantic"
        } else if code < 800 {
            "capability-binding"
        } else {
            "generic"
        }
    }

    /// Returns `true` if this error code represents a warning rather than a hard error.
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            ErrorCode::InvPlaceholderBounds
                | ErrorCode::ParameterArityMismatch
                | ErrorCode::InvAcpAgentNotRegistered
                | ErrorCode::ModelNotFound
                | ErrorCode::CapabilityNotRegistered
                | ErrorCode::DeadNode
                | ErrorCode::CommunicateBeforeSpawn
                | ErrorCode::EmptyTemplate
                | ErrorCode::UncheckedMemoryRead
                | ErrorCode::UnusedCapability
                | ErrorCode::SchemaDrift
        )
    }

    /// Get documentation URL
    pub fn documentation_url(&self) -> String {
        format!("https://apxm.dev/errors/{}", self.as_str())
    }

    /// Convert from u32 (for FFI)
    pub fn from_u32(code: u32) -> Option<Self> {
        match code {
            1 => Some(ErrorCode::UnexpectedToken),
            2 => Some(ErrorCode::ExpectedExpression),
            3 => Some(ErrorCode::ExpectedIdentifier),
            4 => Some(ErrorCode::InvalidNumberLiteral),
            5 => Some(ErrorCode::InvalidStringLiteral),
            6 => Some(ErrorCode::UnknownKeyword),
            7 => Some(ErrorCode::MissingClosingBrace),
            8 => Some(ErrorCode::MissingClosingParen),
            9 => Some(ErrorCode::MissingClosingBracket),
            10 => Some(ErrorCode::DuplicateDeclaration),
            11 => Some(ErrorCode::ExpectedTypeAnnotation),
            12 => Some(ErrorCode::InvalidMemoryTier),
            13 => Some(ErrorCode::ExpectedEventType),
            14 => Some(ErrorCode::ExpectedCapabilityName),
            15 => Some(ErrorCode::ExpectedFlowName),
            16 => Some(ErrorCode::ExpectedBeliefName),
            17 => Some(ErrorCode::ExpectedGoalName),
            18 => Some(ErrorCode::ExpectedAgentName),
            19 => Some(ErrorCode::ExpectedMemoryName),
            20 => Some(ErrorCode::InvalidOperator),
            21 => Some(ErrorCode::ExpectedFunctionName),
            22 => Some(ErrorCode::ExpectedMemberName),
            23 => Some(ErrorCode::ExpectedArrayIndex),
            24 => Some(ErrorCode::ExpectedParameter),
            25 => Some(ErrorCode::ExpectedReturnValue),
            26 => Some(ErrorCode::ExpectedCondition),
            27 => Some(ErrorCode::ExpectedLoopVariable),
            28 => Some(ErrorCode::ExpectedCollection),
            29 => Some(ErrorCode::ExpectedCodeString),
            30 => Some(ErrorCode::ExpectedTraceId),
            31 => Some(ErrorCode::ExpectedGoalString),
            32 => Some(ErrorCode::ExpectedTemplateString),
            33 => Some(ErrorCode::ExpectedRecipient),
            34 => Some(ErrorCode::SyntaxError),
            101 => Some(ErrorCode::TypeMismatch),
            102 => Some(ErrorCode::UndefinedVariable),
            103 => Some(ErrorCode::InvalidTypeAnnotation),
            104 => Some(ErrorCode::TypeInferenceFailed),
            105 => Some(ErrorCode::TypeNotFound),
            106 => Some(ErrorCode::InvalidTypeConversion),
            107 => Some(ErrorCode::TypeAnnotationRequired),
            201 => Some(ErrorCode::MLIRVerificationFailed),
            202 => Some(ErrorCode::InvalidOperation),
            203 => Some(ErrorCode::DagCycleDetected),
            204 => Some(ErrorCode::MissingRequiredOperand),
            205 => Some(ErrorCode::InvalidOperandType),
            206 => Some(ErrorCode::OperationNotFound),
            207 => Some(ErrorCode::InvalidOperationResult),
            301 => Some(ErrorCode::PassExecutionFailed),
            302 => Some(ErrorCode::OptimizationConflict),
            303 => Some(ErrorCode::PassNotFound),
            304 => Some(ErrorCode::PassDependencyFailed),
            401 => Some(ErrorCode::SchedulerError),
            402 => Some(ErrorCode::OperationExecutionFailed),
            403 => Some(ErrorCode::Timeout),
            404 => Some(ErrorCode::CapabilityNotFound),
            405 => Some(ErrorCode::MemoryAccessError),
            406 => Some(ErrorCode::LLMBackendError),
            501 => Some(ErrorCode::TemplatePlaceholderBounds),
            502 => Some(ErrorCode::CommunicateRecipientNotSpawned),
            503 => Some(ErrorCode::DuplicateAgentName),
            504 => Some(ErrorCode::InvPlaceholderBounds),
            505 => Some(ErrorCode::ParameterArityMismatch),
            506 => Some(ErrorCode::ProfileNotRegistered),
            507 => Some(ErrorCode::InvAcpAgentNotRegistered),
            508 => Some(ErrorCode::BackendNotRegistered),
            509 => Some(ErrorCode::ModelNotFound),
            510 => Some(ErrorCode::CapabilityNotRegistered),
            512 => Some(ErrorCode::DeadNode),
            513 => Some(ErrorCode::MissingReturnValue),
            514 => Some(ErrorCode::CommunicateBeforeSpawn),
            515 => Some(ErrorCode::CircularModelProfile),
            516 => Some(ErrorCode::EmptyTemplate),
            517 => Some(ErrorCode::UncheckedMemoryRead),
            518 => Some(ErrorCode::ConstStrWithDynamicInput),
            712 => Some(ErrorCode::UnboundCapability),
            721 => Some(ErrorCode::UnusedCapability),
            722 => Some(ErrorCode::SchemaDrift),
            900 => Some(ErrorCode::InternalError),
            901 => Some(ErrorCode::NotImplemented),
            902 => Some(ErrorCode::InvalidConfiguration),
            903 => Some(ErrorCode::IoError),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
