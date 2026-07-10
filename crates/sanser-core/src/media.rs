use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    #[default]
    Auto,
    H264,
    Hevc,
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "auto",
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        })
    }
}

impl FromStr for VideoCodec {
    type Err = ParseCodecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "h264" | "h.264" | "avc" => Ok(Self::H264),
            "hevc" | "h265" | "h.265" => Ok(Self::Hevc),
            _ => Err(ParseCodecError),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    #[default]
    Opus,
    Pcm16,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("unsupported codec")]
pub struct ParseCodecError;
