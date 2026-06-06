use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Tun(String),
    Crypto(String),
    Handshake(String),
    Protocol(String),
    Config(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO error: {}", e),
            Error::Tun(s) => write!(f, "TUN error: {}", s),
            Error::Crypto(s) => write!(f, "Crypto error: {}", s),
            Error::Handshake(s) => write!(f, "Handshake error: {}", s),
            Error::Protocol(s) => write!(f, "Protocol error: {}", s),
            Error::Config(s) => write!(f, "Config error: {}", s),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
