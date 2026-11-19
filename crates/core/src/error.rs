use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// Transport-level failure (DNS, TLS, timeout, ...).
    Network(String),
    /// The server answered with a non-success status.
    Status {
        code: u16,
        url: String,
    },
    /// GitHub (or another source) told us to slow down.
    RateLimited {
        source: &'static str,
        hint: String,
    },
    NotFound(String),
    Parse(String),
    Io(std::io::Error),
    /// Downloaded file does not match the digest published by the source.
    DigestMismatch {
        expected: String,
        actual: String,
    },
    Unsupported(String),
    /// The user stopped a download.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(m) => write!(f, "{m}"),
            Error::Status { code, url } => write!(f, "HTTP {code} from {url}"),
            Error::RateLimited { source, hint } => write!(f, "{source} rate limit reached. {hint}"),
            Error::NotFound(what) => write!(f, "not found: {what}"),
            Error::Parse(m) => write!(f, "unexpected response: {m}"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::DigestMismatch { expected, actual } => {
                write!(f, "checksum mismatch (expected {expected}, got {actual}); file deleted")
            }
            Error::Unsupported(m) => write!(f, "{m}"),
            Error::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Parse(e.to_string())
    }
}
