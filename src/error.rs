//! Error type that carries a process exit code.

use std::fmt;

/// Exit codes from the plan's CLI contract.
pub mod exit {
    pub const OK: i32 = 0;
    pub const USAGE: i32 = 2;
    pub const NEEDS_HUMAN: i32 = 3;
    pub const FAILED: i32 = 4;
    pub const CONCURRENCY: i32 = 5;
}

/// An error with an exit code. Anything else that bubbles up as `anyhow::Error`
/// exits with [`exit::FAILED`].
#[derive(Debug)]
pub struct MusterError {
    pub code: i32,
    pub message: String,
}

impl MusterError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn config(message: impl Into<String>) -> Self {
        Self::new(exit::USAGE, message)
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(exit::USAGE, message)
    }
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(exit::FAILED, message)
    }
}

impl fmt::Display for MusterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MusterError {}

/// Exit code for an error chain: the first `MusterError` found, else FAILED.
pub fn exit_code(err: &anyhow::Error) -> i32 {
    err.chain()
        .find_map(|e| e.downcast_ref::<MusterError>())
        .map(|e| e.code)
        .unwrap_or(exit::FAILED)
}
