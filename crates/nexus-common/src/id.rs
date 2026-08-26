//! Strongly typed entity IDs for the Nexus platform.
//!
//! Provides validated, OS-independent string ID newtypes for devices, users,
//! nodes, tenants, sessions, and clients.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use thiserror::Error;

/// Minimum length of an entity ID in bytes.
pub const MIN_ID_LEN: usize = 1;

/// Maximum length of an entity ID in bytes.
pub const MAX_ID_LEN: usize = 128;

/// Error returned when ID validation fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    /// The ID string was empty.
    #[error("ID cannot be empty")]
    Empty,

    /// The ID string length exceeded the maximum allowable length.
    #[error("ID length {actual} exceeds maximum allowed length of {max}")]
    TooLong {
        /// Maximum allowed length in bytes.
        max: usize,
        /// Actual length of the provided string in bytes.
        actual: usize,
    },

    /// The ID string contained an invalid character (non-ASCII printable or control character).
    #[error("ID contains invalid character '{c:?}' at byte offset {offset}")]
    InvalidCharacter {
        /// The invalid character found.
        c: char,
        /// Byte offset in the input string where the invalid character occurred.
        offset: usize,
    },
}

/// Validates an ID string according to length and character safety rules.
///
/// Rules:
/// - Length must be between 1 and 128 bytes inclusive.
/// - Characters must be printable ASCII (`0x20..=0x7E`) and non-control characters.
pub fn validate_id(s: &str) -> Result<(), IdError> {
    if s.is_empty() {
        return Err(IdError::Empty);
    }

    if s.len() > MAX_ID_LEN {
        return Err(IdError::TooLong {
            max: MAX_ID_LEN,
            actual: s.len(),
        });
    }

    for (offset, c) in s.char_indices() {
        if !c.is_ascii() || c.is_ascii_control() {
            return Err(IdError::InvalidCharacter { c, offset });
        }
    }

    Ok(())
}

/// Macro to define a strongly-typed entity ID newtype with standard traits and validation.
#[macro_export]
macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Creates a new validated ID.
            ///
            /// Returns an [`IdError`] if validation fails (empty, > 128 bytes, or non-printable ASCII).
            pub fn new(id: impl Into<String>) -> Result<Self, $crate::id::IdError> {
                let s = id.into();
                $crate::id::validate_id(&s)?;
                Ok(Self(s))
            }

            /// Returns the ID as a string slice.
            #[inline]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consumes the ID and returns the underlying [`String`].
            #[inline]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl $crate::id::Deref for $name {
            type Target = str;

            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl $crate::id::Borrow<str> for $name {
            #[inline]
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            #[inline]
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl $crate::id::fmt::Display for $name {
            fn fmt(&self, f: &mut $crate::id::fmt::Formatter<'_>) -> $crate::id::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl $crate::id::FromStr for $name {
            type Err = $crate::id::IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = $crate::id::IdError;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::new(s)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = $crate::id::IdError;

            fn try_from(s: &str) -> Result<Self, Self::Error> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            #[inline]
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                Self::new(s).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_id!(DeviceId, "Strongly-typed unique identifier for a device.");
define_id!(UserId, "Strongly-typed unique identifier for a user.");
define_id!(
    NodeId,
    "Strongly-typed unique identifier for a cluster/relay node."
);
define_id!(
    TenantId,
    "Strongly-typed unique identifier for an organization or tenant."
);
define_id!(
    SessionId,
    "Strongly-typed unique identifier for a remote desktop session."
);
define_id!(
    ClientId,
    "Strongly-typed unique identifier for a client connection or client instance."
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn test_valid_id_creation_and_access() {
        let dev_id = DeviceId::new("dev-12345").expect("valid device ID");
        assert_eq!(dev_id.as_str(), "dev-12345");
        assert_eq!(dev_id.to_string(), "dev-12345");
        assert_eq!(&*dev_id, "dev-12345");
        assert_eq!(dev_id.as_ref(), "dev-12345");
        assert_eq!(dev_id.into_inner(), "dev-12345");
    }

    #[test]
    fn test_all_id_types_creation() {
        assert_eq!(DeviceId::new("dev-1").unwrap().as_str(), "dev-1");
        assert_eq!(UserId::new("usr-1").unwrap().as_str(), "usr-1");
        assert_eq!(NodeId::new("node-1").unwrap().as_str(), "node-1");
        assert_eq!(TenantId::new("tenant-1").unwrap().as_str(), "tenant-1");
        assert_eq!(SessionId::new("session-1").unwrap().as_str(), "session-1");
        assert_eq!(ClientId::new("client-1").unwrap().as_str(), "client-1");
    }

    #[test]
    fn test_min_and_max_bounds() {
        // Min bound: 1 byte
        let min_id = DeviceId::new("a");
        assert!(min_id.is_ok());
        assert_eq!(min_id.unwrap().as_str(), "a");

        // Max bound: 128 bytes
        let max_str = "a".repeat(128);
        let max_id = DeviceId::new(&max_str);
        assert!(max_id.is_ok());
        assert_eq!(max_id.unwrap().as_str(), &max_str);

        // Underflow: empty string
        let empty_res = DeviceId::new("");
        assert_eq!(empty_res, Err(IdError::Empty));

        // Overflow: 129 bytes
        let overflow_str = "a".repeat(129);
        let overflow_res = DeviceId::new(&overflow_str);
        assert_eq!(
            overflow_res,
            Err(IdError::TooLong {
                max: 128,
                actual: 129
            })
        );
    }

    #[test]
    fn test_invalid_characters() {
        // Null byte
        assert_eq!(
            DeviceId::new("dev\0id"),
            Err(IdError::InvalidCharacter { c: '\0', offset: 3 })
        );

        // Newline
        assert_eq!(
            UserId::new("user\nid"),
            Err(IdError::InvalidCharacter { c: '\n', offset: 4 })
        );

        // Tab
        assert_eq!(
            NodeId::new("node\tid"),
            Err(IdError::InvalidCharacter { c: '\t', offset: 4 })
        );

        // Carriage return
        assert_eq!(
            SessionId::new("session\rid"),
            Err(IdError::InvalidCharacter { c: '\r', offset: 7 })
        );

        // Non-ASCII UTF-8 character (emoji)
        assert_eq!(
            TenantId::new("tenant-🦀"),
            Err(IdError::InvalidCharacter {
                c: '🦀', offset: 7
            })
        );

        // Non-ASCII UTF-8 character (accented)
        assert_eq!(
            ClientId::new("client-é"),
            Err(IdError::InvalidCharacter { c: 'é', offset: 7 })
        );
    }

    #[test]
    fn test_printable_ascii_allowed() {
        let valid_symbols = "dev-123_ABC.xyz:foo~bar@domain";
        let id = DeviceId::new(valid_symbols);
        assert!(id.is_ok());
        assert_eq!(id.unwrap().as_str(), valid_symbols);
    }

    #[test]
    fn test_display_and_from_str_roundtrip() {
        let original_str = "sess-abcdef-98765";
        let parsed: SessionId = original_str.parse().unwrap();
        assert_eq!(parsed.as_str(), original_str);
        assert_eq!(parsed.to_string(), original_str);

        let invalid_parsed = "".parse::<SessionId>();
        assert_eq!(invalid_parsed, Err(IdError::Empty));
    }

    #[test]
    fn test_try_from_conversions() {
        let id_from_str = DeviceId::try_from("dev-123").unwrap();
        assert_eq!(id_from_str.as_str(), "dev-123");

        let id_from_string = DeviceId::try_from(String::from("dev-123")).unwrap();
        assert_eq!(id_from_string.as_str(), "dev-123");

        let string_back: String = id_from_string.into();
        assert_eq!(string_back, "dev-123");

        assert_eq!(DeviceId::try_from(""), Err(IdError::Empty));
    }

    #[test]
    fn test_hash_and_equality() {
        let id1 = ClientId::new("client-abc").unwrap();
        let id2 = ClientId::new("client-abc").unwrap();
        let id3 = ClientId::new("client-xyz").unwrap();

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }

    #[test]
    fn test_ordering() {
        let id1 = UserId::new("user-1").unwrap();
        let id2 = UserId::new("user-2").unwrap();
        let id3 = UserId::new("user-3").unwrap();

        let mut set = BTreeSet::new();
        set.insert(id2.clone());
        set.insert(id1.clone());
        set.insert(id3.clone());

        let ordered: Vec<UserId> = set.into_iter().collect();
        assert_eq!(ordered, vec![id1, id2, id3]);
    }

    #[test]
    fn test_serde_json_serialization_and_deserialization() {
        let dev_id = DeviceId::new("device-999").unwrap();
        let json_str = serde_json::to_string(&dev_id).unwrap();
        assert_eq!(json_str, "\"device-999\"");

        let deserialized: DeviceId = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized, dev_id);
    }

    #[test]
    fn test_serde_json_deserialization_validation_failure() {
        // Empty string
        let empty_json = "\"\"";
        let res: Result<DeviceId, _> = serde_json::from_str(empty_json);
        assert!(res.is_err());

        // Too long
        let too_long_json = format!("\"{}\"", "x".repeat(129));
        let res: Result<DeviceId, _> = serde_json::from_str(&too_long_json);
        assert!(res.is_err());

        // Control char
        let invalid_json = "\"dev\\u0000ice\"";
        let res: Result<DeviceId, _> = serde_json::from_str(invalid_json);
        assert!(res.is_err());
    }
}
