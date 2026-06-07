use std::error::Error;
use std::fmt::{Display, Formatter};

pub type MemoryResult<T> = Result<T, MemoryError>;
pub type CapacityError = MemoryError;
pub type PinError = MemoryError;
pub type PublishError = MemoryError;
pub type StoreDeleteError = MemoryError;
pub type StoreWriteError = MemoryError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryError {
    Build(String),
    Config(String),
    Capacity(String),
    InvalidState(String),
    Report(String),
    Unsupported(String),
}

impl MemoryError {
    pub fn build(message: impl Into<String>) -> Self {
        Self::Build(message.into())
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    pub fn capacity(message: impl Into<String>) -> Self {
        Self::Capacity(message.into())
    }

    pub fn report(message: impl Into<String>) -> Self {
        Self::Report(message.into())
    }
}

impl Display for MemoryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(message) => write!(f, "build error: {message}"),
            Self::Config(message) => write!(f, "config error: {message}"),
            Self::Capacity(message) => write!(f, "capacity error: {message}"),
            Self::InvalidState(message) => write!(f, "invalid state: {message}"),
            Self::Report(message) => write!(f, "report error: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl Error for MemoryError {}
