//! Token-based dataflow representation for APXM.
//!
//! Tokens represent values in the dataflow execution model.
//! They track the state of values as they flow through the execution DAG.

use serde::{Deserialize, Serialize};

use crate::error::runtime::RuntimeError;

use super::{TokenId, Value};

/// Represents the state of a token in the dataflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenStatus {
    /// Token is waiting for a value.
    Pending,
    /// Token has a value and is ready to be consumed.
    Ready,
    /// Token has been consumed by all consumers.
    Consumed,
}

/// Represents a token in the dataflow execution model.
///
/// Tokens carry values through the exection DAG and track their state
/// to enable data-driven execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Token {
    /// Unique identifier for this token.
    pub id: TokenId,
    /// The value carried by this token (only when Ready).
    pub value: Option<Value>,
    /// Current status of the token.
    pub status: TokenStatus,
}

impl Token {
    /// Create a new token in Pending state.
    ///
    /// # Examples
    ///
    /// ```
    /// use apxm_core::types::{Token, TokenIdType};
    ///
    /// let token = Token::new(1);
    /// assert!(!token.is_ready());
    /// ```
    pub fn new(id: TokenId) -> Self {
        Token {
            id,
            value: None,
            status: TokenStatus::Pending,
        }
    }

    /// Checks if the token is ready (has a value).
    pub fn is_ready(&self) -> bool {
        matches!(self.status, TokenStatus::Ready)
    }

    /// Sets the value for this token and marks it as Ready
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not in Pending state.
    pub fn set_value(&mut self, value: Value) -> Result<(), RuntimeError> {
        match self.status {
            TokenStatus::Pending => {
                self.value = Some(value);
                self.status = TokenStatus::Ready;
                Ok(())
            }
            TokenStatus::Ready => Err(RuntimeError::State("Token is already ready".to_string())),
            TokenStatus::Consumed => {
                Err(RuntimeError::State("Token is already consumed".to_string()))
            }
        }
    }

    /// Consumes the token and returns its value.
    ///
    /// This marks the token as Consumed and returns the value.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not in Ready state.
    pub fn consume(&mut self) -> Result<Value, RuntimeError> {
        match self.status {
            TokenStatus::Ready => {
                let value = self
                    .value
                    .take()
                    .ok_or_else(|| RuntimeError::State("Token has no value".to_string()))?;
                self.status = TokenStatus::Consumed;
                Ok(value)
            }
            TokenStatus::Pending => Err(RuntimeError::State("Token is not ready".to_string())),
            TokenStatus::Consumed => {
                Err(RuntimeError::State("Token is already consumed".to_string()))
            }
        }
    }

    /// Gets a reference to the token's value without consuming it.
    ///
    /// Returns `None` if the token is not ready.
    pub fn get_value(&self) -> Option<&Value> {
        if self.is_ready() {
            self.value.as_ref()
        } else {
            None
        }
    }
}

