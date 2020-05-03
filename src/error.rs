use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("problem with the database")]
    SledError {
        #[from]
        source: sled::Error,
    },
    #[error("tried to parse invalid string")]
    StringFromBytesError {
        #[from]
        source: std::string::FromUtf8Error,
    },
    #[error("tried to parse invalid identifier")]
    SerdeJsonError {
        #[from]
        source: serde_json::Error,
    },
    #[error("tried to parse invalid identifier")]
    RumaIdentifierError {
        #[from]
        source: ruma_identifiers::Error,
    },
    #[error("tried to parse invalid event")]
    RumaEventError {
        #[from]
        source: ruma_events::InvalidEvent,
    },
    #[error("bad request")]
    BadRequest(&'static str),
    #[error("problem in that database")]
    BadDatabase(&'static str),
}
