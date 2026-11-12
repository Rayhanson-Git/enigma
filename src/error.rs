//! Error kinds and messages (00 §5).
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    VaultError,
    NoteExists,
    LockBusy,
    ProtectedLink,
    SheetError,
    ExtractorError,
    InvalidArgument,
    DecodeError,
    InvalidData,
    IoError,
    NotFound,
    EvalConfigError,
    ProvisionError,
    SyncError,
    Internal,
}

impl Kind {
    fn parent(self) -> Option<Kind> {
        use Kind::*;
        match self {
            NoteExists | LockBusy | ProtectedLink => Some(VaultError),
            DecodeError | InvalidData => Some(InvalidArgument),
            NotFound => Some(IoError),
            _ => None,
        }
    }

    /// True when self is `k` or one of its sub-kinds.
    pub fn is(self, k: Kind) -> bool {
        self == k || self.parent() == Some(k)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: Kind,
    pub msg: String,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(kind: Kind, msg: impl Into<String>) -> Self {
        Error { kind, msg: msg.into() }
    }
    pub fn vault(msg: impl Into<String>) -> Self {
        Self::new(Kind::VaultError, msg)
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(Kind::InvalidArgument, msg)
    }
    /// An OS error on `path`: NotFound when the file is absent, IoError otherwise.
    pub fn io(e: &std::io::Error, path: &Path) -> Self {
        let kind = if e.kind() == std::io::ErrorKind::NotFound { Kind::NotFound } else { Kind::IoError };
        Self::new(kind, format!("{e}: {}", path.display()))
    }
    pub fn is(&self, k: Kind) -> bool {
        self.kind.is(k)
    }
}

impl fmt::Display for Error {
    /// The "<Kind>: <message>" rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.msg)
    }
}

impl std::error::Error for Error {}
