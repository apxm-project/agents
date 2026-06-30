//! Observability primitives shared between compiler, runtime, and backends.

pub mod call_trace;

pub use call_trace::{CallEvent, CallTrace};
