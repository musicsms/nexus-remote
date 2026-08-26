//! Shared error hierarchy for Nexus Remote Desktop Platform.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

use crate::id::IdError;

/// Standardized error codes for Nexus operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request does not have valid authentication credentials.
    #[serde(alias = "Unauthenticated", alias = "UNAUTHENTICATED")]
    Unauthenticated,

    /// The caller does not have permission to execute the specified operation.
    #[serde(alias = "PermissionDenied", alias = "PERMISSION_DENIED")]
    PermissionDenied,

    /// Some requested entity was not found.
    #[serde(alias = "NotFound", alias = "NOT_FOUND")]
    NotFound,

    /// An entity that we attempted to create already exists.
    #[serde(alias = "AlreadyExists", alias = "ALREADY_EXISTS")]
    AlreadyExists,

    /// Client specified an invalid argument.
    #[serde(alias = "InvalidArgument", alias = "INVALID_ARGUMENT")]
    InvalidArgument,

    /// Deadline expired before operation could complete.
    #[serde(alias = "DeadlineExceeded", alias = "DEADLINE_EXCEEDED")]
    DeadlineExceeded,

    /// Concurrency conflict or state mismatch.
    #[serde(alias = "Conflict", alias = "CONFLICT")]
    Conflict,

    /// Some resource has been exhausted, e.g. quota or rate limit.
    #[serde(alias = "ResourceExhausted", alias = "RESOURCE_EXHAUSTED")]
    ResourceExhausted,

    /// Internal system error.
    #[serde(alias = "Internal", alias = "INTERNAL")]
    Internal,

    /// The service is currently unavailable (e.g. relay down or network partition).
    #[serde(alias = "Unavailable", alias = "UNAVAILABLE")]
    Unavailable,

    /// Operation is not implemented or not supported.
    #[serde(alias = "NotSupported", alias = "NOT_SUPPORTED")]
    NotSupported,
}

impl ErrorCode {
    /// Returns the static string slice representing this error code in snake_case.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::PermissionDenied => "permission_denied",
            Self::NotFound => "not_found",
            Self::AlreadyExists => "already_exists",
            Self::InvalidArgument => "invalid_argument",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Conflict => "conflict",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Internal => "internal",
            Self::Unavailable => "unavailable",
            Self::NotSupported => "not_supported",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an invalid error code string.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown error code: '{0}'")]
pub struct ParseErrorCodeError(pub String);

impl FromStr for ErrorCode {
    type Err = ParseErrorCodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase().replace(['_', '-'], "");
        match normalized.as_str() {
            "unauthenticated" => Ok(Self::Unauthenticated),
            "permissiondenied" => Ok(Self::PermissionDenied),
            "notfound" => Ok(Self::NotFound),
            "alreadyexists" => Ok(Self::AlreadyExists),
            "invalidargument" => Ok(Self::InvalidArgument),
            "deadlineexceeded" => Ok(Self::DeadlineExceeded),
            "conflict" => Ok(Self::Conflict),
            "resourceexhausted" => Ok(Self::ResourceExhausted),
            "internal" => Ok(Self::Internal),
            "unavailable" => Ok(Self::Unavailable),
            "notsupported" => Ok(Self::NotSupported),
            _ => Err(ParseErrorCodeError(s.to_string())),
        }
    }
}

/// Common structured error type for Nexus platform crates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Error)]
#[error("{code}: {message}")]
pub struct CommonError {
    /// Machine-readable error code.
    pub code: ErrorCode,
    /// Human-readable error message.
    pub message: String,
}

impl CommonError {
    /// Creates a new `CommonError` with the given code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Creates an `Unauthenticated` error.
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unauthenticated, message)
    }

    /// Creates a `PermissionDenied` error.
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PermissionDenied, message)
    }

    /// Creates a `NotFound` error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    /// Creates an `AlreadyExists` error.
    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AlreadyExists, message)
    }

    /// Creates an `InvalidArgument` error.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    /// Creates a `DeadlineExceeded` error.
    pub fn deadline_exceeded(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DeadlineExceeded, message)
    }

    /// Creates a `Conflict` error.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    /// Creates a `ResourceExhausted` error.
    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ResourceExhausted, message)
    }

    /// Creates an `Internal` error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// Creates an `Unavailable` error.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }

    /// Creates a `NotSupported` error.
    pub fn not_supported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotSupported, message)
    }

    /// Returns the error code.
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// Returns a reference to the error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<IdError> for CommonError {
    fn from(err: IdError) -> Self {
        Self::invalid_argument(err.to_string())
    }
}

impl From<ParseErrorCodeError> for CommonError {
    fn from(err: ParseErrorCodeError) -> Self {
        Self::invalid_argument(err.to_string())
    }
}

impl From<std::io::Error> for CommonError {
    fn from(err: std::io::Error) -> Self {
        let code = match err.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            std::io::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
                ErrorCode::InvalidArgument
            }
            std::io::ErrorKind::TimedOut => ErrorCode::DeadlineExceeded,
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::AddrInUse
            | std::io::ErrorKind::AddrNotAvailable
            | std::io::ErrorKind::BrokenPipe => ErrorCode::Unavailable,
            std::io::ErrorKind::Unsupported => ErrorCode::NotSupported,
            _ => ErrorCode::Internal,
        };
        Self::new(code, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_error_code_as_str_and_display() {
        let cases = [
            (ErrorCode::Unauthenticated, "unauthenticated"),
            (ErrorCode::PermissionDenied, "permission_denied"),
            (ErrorCode::NotFound, "not_found"),
            (ErrorCode::AlreadyExists, "already_exists"),
            (ErrorCode::InvalidArgument, "invalid_argument"),
            (ErrorCode::DeadlineExceeded, "deadline_exceeded"),
            (ErrorCode::Conflict, "conflict"),
            (ErrorCode::ResourceExhausted, "resource_exhausted"),
            (ErrorCode::Internal, "internal"),
            (ErrorCode::Unavailable, "unavailable"),
            (ErrorCode::NotSupported, "not_supported"),
        ];

        for (code, expected) in cases {
            assert_eq!(code.as_str(), expected);
            assert_eq!(code.to_string(), expected);
        }
    }

    #[test]
    fn test_error_code_from_str() {
        let valid_cases = [
            ("unauthenticated", ErrorCode::Unauthenticated),
            ("Unauthenticated", ErrorCode::Unauthenticated),
            ("UNAUTHENTICATED", ErrorCode::Unauthenticated),
            ("permission_denied", ErrorCode::PermissionDenied),
            ("PermissionDenied", ErrorCode::PermissionDenied),
            ("PERMISSION_DENIED", ErrorCode::PermissionDenied),
            ("permission-denied", ErrorCode::PermissionDenied),
            ("not_found", ErrorCode::NotFound),
            ("NotFound", ErrorCode::NotFound),
            ("NOT_FOUND", ErrorCode::NotFound),
            ("not-found", ErrorCode::NotFound),
            ("already_exists", ErrorCode::AlreadyExists),
            ("AlreadyExists", ErrorCode::AlreadyExists),
            ("ALREADY_EXISTS", ErrorCode::AlreadyExists),
            ("invalid_argument", ErrorCode::InvalidArgument),
            ("InvalidArgument", ErrorCode::InvalidArgument),
            ("INVALID_ARGUMENT", ErrorCode::InvalidArgument),
            ("deadline_exceeded", ErrorCode::DeadlineExceeded),
            ("DeadlineExceeded", ErrorCode::DeadlineExceeded),
            ("DEADLINE_EXCEEDED", ErrorCode::DeadlineExceeded),
            ("conflict", ErrorCode::Conflict),
            ("Conflict", ErrorCode::Conflict),
            ("CONFLICT", ErrorCode::Conflict),
            ("resource_exhausted", ErrorCode::ResourceExhausted),
            ("ResourceExhausted", ErrorCode::ResourceExhausted),
            ("RESOURCE_EXHAUSTED", ErrorCode::ResourceExhausted),
            ("internal", ErrorCode::Internal),
            ("Internal", ErrorCode::Internal),
            ("INTERNAL", ErrorCode::Internal),
            ("unavailable", ErrorCode::Unavailable),
            ("Unavailable", ErrorCode::Unavailable),
            ("UNAVAILABLE", ErrorCode::Unavailable),
            ("not_supported", ErrorCode::NotSupported),
            ("NotSupported", ErrorCode::NotSupported),
            ("NOT_SUPPORTED", ErrorCode::NotSupported),
        ];

        for (input, expected) in valid_cases {
            assert_eq!(input.parse::<ErrorCode>().unwrap(), expected);
            assert_eq!(ErrorCode::from_str(input).unwrap(), expected);
        }

        let invalid = "unknown_code".parse::<ErrorCode>();
        assert!(invalid.is_err());
        assert_eq!(
            invalid.unwrap_err(),
            ParseErrorCodeError("unknown_code".to_string())
        );
    }

    #[test]
    fn test_error_code_serde_json_roundtrip() {
        let codes = [
            ErrorCode::Unauthenticated,
            ErrorCode::PermissionDenied,
            ErrorCode::NotFound,
            ErrorCode::AlreadyExists,
            ErrorCode::InvalidArgument,
            ErrorCode::DeadlineExceeded,
            ErrorCode::Conflict,
            ErrorCode::ResourceExhausted,
            ErrorCode::Internal,
            ErrorCode::Unavailable,
            ErrorCode::NotSupported,
        ];

        for code in codes {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{}\"", code.as_str()));
            let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, code);
        }

        // Test deserializing from PascalCase and UPPERCASE
        let from_pascal: ErrorCode = serde_json::from_str("\"NotFound\"").unwrap();
        assert_eq!(from_pascal, ErrorCode::NotFound);

        let from_upper: ErrorCode = serde_json::from_str("\"PERMISSION_DENIED\"").unwrap();
        assert_eq!(from_upper, ErrorCode::PermissionDenied);
    }

    #[test]
    fn test_common_error_constructors_and_accessors() {
        let err = CommonError::invalid_argument("param x is invalid");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert_eq!(err.message(), "param x is invalid");

        let err = CommonError::not_found("device dev-1 not found");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert_eq!(err.message(), "device dev-1 not found");

        let err = CommonError::permission_denied("access denied");
        assert_eq!(err.code(), ErrorCode::PermissionDenied);
        assert_eq!(err.message(), "access denied");

        let err = CommonError::unauthenticated("missing token");
        assert_eq!(err.code(), ErrorCode::Unauthenticated);
        assert_eq!(err.message(), "missing token");

        let err = CommonError::internal("database error");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(err.message(), "database error");

        let err = CommonError::conflict("session already active");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(err.message(), "session already active");

        let err = CommonError::unavailable("relay unreachable");
        assert_eq!(err.code(), ErrorCode::Unavailable);
        assert_eq!(err.message(), "relay unreachable");

        let err = CommonError::already_exists("user exists");
        assert_eq!(err.code(), ErrorCode::AlreadyExists);
        assert_eq!(err.message(), "user exists");

        let err = CommonError::deadline_exceeded("operation timed out");
        assert_eq!(err.code(), ErrorCode::DeadlineExceeded);
        assert_eq!(err.message(), "operation timed out");

        let err = CommonError::resource_exhausted("rate limit exceeded");
        assert_eq!(err.code(), ErrorCode::ResourceExhausted);
        assert_eq!(err.message(), "rate limit exceeded");

        let err = CommonError::not_supported("feature not supported");
        assert_eq!(err.code(), ErrorCode::NotSupported);
        assert_eq!(err.message(), "feature not supported");
    }

    #[test]
    fn test_common_error_display_and_error_trait() {
        let err = CommonError::not_found("session ses-123");
        assert_eq!(format!("{err}"), "not_found: session ses-123");

        let std_err: &dyn std::error::Error = &err;
        assert_eq!(format!("{std_err}"), "not_found: session ses-123");
    }

    #[test]
    fn test_common_error_serde_json_roundtrip() {
        let err = CommonError::permission_denied("role admin required");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(
            json,
            r#"{"code":"permission_denied","message":"role admin required"}"#
        );

        let deserialized: CommonError = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, err);
        assert_eq!(deserialized.code(), ErrorCode::PermissionDenied);
        assert_eq!(deserialized.message(), "role admin required");
    }

    #[test]
    fn test_common_error_hash_and_equality() {
        let err1 = CommonError::invalid_argument("same message");
        let err2 = CommonError::invalid_argument("same message");
        let err3 = CommonError::not_found("same message");
        let err4 = CommonError::invalid_argument("different message");

        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
        assert_ne!(err1, err4);

        let mut set = HashSet::new();
        set.insert(err1.clone());
        assert!(set.contains(&err2));
        assert!(!set.contains(&err3));
        assert!(!set.contains(&err4));
    }

    #[test]
    fn test_from_id_error() {
        let id_err = IdError::Empty;
        let common_err: CommonError = id_err.clone().into();
        assert_eq!(common_err.code(), ErrorCode::InvalidArgument);
        assert_eq!(common_err.message(), id_err.to_string());

        let id_err2 = IdError::TooLong {
            max: 128,
            actual: 200,
        };
        let common_err2: CommonError = id_err2.clone().into();
        assert_eq!(common_err2.code(), ErrorCode::InvalidArgument);
        assert_eq!(common_err2.message(), id_err2.to_string());

        let id_err3 = IdError::InvalidCharacter { c: '\0', offset: 2 };
        let common_err3: CommonError = id_err3.clone().into();
        assert_eq!(common_err3.code(), ErrorCode::InvalidArgument);
        assert_eq!(common_err3.message(), id_err3.to_string());
    }

    #[test]
    fn test_from_io_error() {
        let io_not_found = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let common_not_found: CommonError = io_not_found.into();
        assert_eq!(common_not_found.code(), ErrorCode::NotFound);
        assert_eq!(common_not_found.message(), "file missing");

        let io_denied =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access forbidden");
        let common_denied: CommonError = io_denied.into();
        assert_eq!(common_denied.code(), ErrorCode::PermissionDenied);

        let io_timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, "socket timeout");
        let common_timeout: CommonError = io_timeout.into();
        assert_eq!(common_timeout.code(), ErrorCode::DeadlineExceeded);

        let io_refused = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let common_refused: CommonError = io_refused.into();
        assert_eq!(common_refused.code(), ErrorCode::Unavailable);
    }

    #[test]
    fn test_from_parse_error_code_error() {
        let parse_err = ParseErrorCodeError("bad_code".to_string());
        let common_err: CommonError = parse_err.into();
        assert_eq!(common_err.code(), ErrorCode::InvalidArgument);
        assert_eq!(common_err.message(), "unknown error code: 'bad_code'");
    }
}
