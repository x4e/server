use crate::permission::{ChannelPermission, HubPermission};
use reqwest::StatusCode;
use thiserror::Error;
use warp::reject::Reject;

/// General result type for wicrs, error type defaults to [`Error`].
pub type Result<T = (), E = Error> = std::result::Result<T, E>;

/// General errors that can occur when using the WICRS API.
#[derive(Debug, Error)]
pub enum Error {
    #[error("user is muted and cannot send messages")]
    Muted,
    #[error("user is banned from that hub")]
    Banned,
    #[error("hub does not exist")]
    HubNotFound,
    #[error("channel does not exist")]
    ChannelNotFound,
    #[error("user is missing the {0} hub permission")]
    MissingHubPermission(HubPermission),
    #[error("user is missing the {0} channel permission")]
    MissingChannelPermission(ChannelPermission),
    #[error("user is not in the hub")]
    NotInHub,
    #[error("member does not exist")]
    MemberNotFound,
    #[error("message does not exist")]
    MessageNotFound,
    #[error("permission group does not exist")]
    GroupNotFound,
    #[error("invalid name")]
    InvalidName,
    #[error("something strange happened")]
    UnexpectedServerArg,
    #[error("text object to big")]
    TooBig,
    #[error("not utf-8 bytes")]
    InvalidText,
    #[error("bad message format")]
    InvalidMessage,
    #[error("user already typing")]
    AlreadyTyping,
    #[error("user not typing")]
    NotTyping,
    #[error("internal server message failed")]
    InternalMessageFailed,
    #[error("internal handler servers failed to start")]
    ServerStartFailed,
    #[error("IO serror")]
    Io(#[from] std::io::Error),
    #[error("JSON error")]
    JSON(#[from] serde_json::Error),
    #[error("Bincode error")]
    Bincode(#[from] bincode::Error),
    #[error("Tantivy error")]
    Tantivy(#[from] tantivy::error::TantivyError),
    #[error("Tantivy error")]
    TantivyOpenDirectory(#[from] tantivy::directory::error::OpenDirectoryError),
    #[error("Tantivy error")]
    TantivyOpenRead(#[from] tantivy::directory::error::OpenReadError),
    #[error("Tantivy error")]
    TantivyOpenWrite(#[from] tantivy::directory::error::OpenWriteError),
    #[error("Tantivy error")]
    TantivyQueryParse(#[from] tantivy::query::QueryParserError),
    #[error("could not get a Tantivy index writer")]
    GetIndexWriter,
    #[error("could not get a Tantivy index reader")]
    GetIndexReader,
    #[error("Warp error")]
    Warp(#[from] warp::Error),
    #[error("PGP error")]
    PGP(#[from] pgp::errors::Error),
    #[error("{0}")]
    Other(String),
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::Other(s)
    }
}

impl Reject for Error {}

impl From<&Error> for StatusCode {
    fn from(error: &Error) -> Self {
        match error {
            Error::InvalidName => Self::BAD_REQUEST,
            Error::Banned => Self::FORBIDDEN,
            Error::ChannelNotFound => Self::NOT_FOUND,
            Error::GroupNotFound => Self::NOT_FOUND,
            Error::HubNotFound => Self::NOT_FOUND,
            Error::MemberNotFound => Self::NOT_FOUND,
            Error::MessageNotFound => Self::NOT_FOUND,
            Error::Muted => Self::FORBIDDEN,
            Error::MissingChannelPermission(_) => Self::FORBIDDEN,
            Error::MissingHubPermission(_) => Self::FORBIDDEN,
            Error::NotInHub => Self::NOT_FOUND,
            Error::TooBig => Self::BAD_REQUEST,
            Error::InvalidText => Self::BAD_REQUEST,
            Error::AlreadyTyping => Self::CONFLICT,
            Error::NotTyping => Self::CONFLICT,
            _ => Self::INTERNAL_SERVER_ERROR,
        }
    }
}
