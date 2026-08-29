use std::error::Error as StdError;
use std::fmt;

/// Failures at the persistence boundary: the `SQLite` Application Store.
///
/// The store raises [`StoreError::InvalidOperation`] for every kind of fault
/// it can encounter — connection setup, migration execution, transaction
/// errors, constraint violations, and IO failures on the database file. The
/// variant carries a human-readable reason string so no infrastructure error
/// type leaks into the application layer.
#[derive(Debug, Clone)]
pub enum StoreError {
    InvalidOperation(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::InvalidOperation(msg) => write!(f, "Invalid operation: {msg}"),
        }
    }
}

impl StdError for StoreError {}
