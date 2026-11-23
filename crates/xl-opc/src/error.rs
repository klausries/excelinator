use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The container is not a readable zip archive.
    Zip(String),
    /// A part contains XML that cannot be read.
    Xml(String),
    /// The package violates a structural rule.
    Invalid(String),
    /// A resource limit was exceeded.
    Limit(String),
    /// The requested part does not exist.
    NotFound(String),
}

impl Error {
    pub(crate) fn zip(e: zip::result::ZipError) -> Self {
        Error::Zip(e.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Zip(m) => write!(f, "zip: {m}"),
            Error::Xml(m) => write!(f, "xml: {m}"),
            Error::Invalid(m) => write!(f, "invalid package: {m}"),
            Error::Limit(m) => write!(f, "limit exceeded: {m}"),
            Error::NotFound(m) => write!(f, "part not found: {m}"),
        }
    }
}

impl std::error::Error for Error {}
