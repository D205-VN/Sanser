use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            #[must_use]
            pub const fn into_uuid(self) -> Uuid {
                self.0
            }

            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl TryFrom<&[u8]> for $name {
            type Error = uuid::Error;

            fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
                Uuid::from_slice(bytes).map(Self)
            }
        }
    };
}

uuid_id!(DeviceId);
uuid_id!(SessionId);

/// Logical media/input stream inside one session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(u32);

impl StreamId {
    pub const CONTROL: Self = Self(0);
    pub const VIDEO_PRIMARY: Self = Self(1);
    pub const AUDIO_PRIMARY: Self = Self(2);
    pub const INPUT: Self = Self(3);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
