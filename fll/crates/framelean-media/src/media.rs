use framelean_core::{EngineError, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rational {
    numerator: i64,
    denominator: u64,
}

impl Rational {
    pub fn new(numerator: i64, denominator: u64) -> Result<Self> {
        if denominator == 0 {
            return Err(EngineError::invalid_argument(
                "rational denominator must be greater than zero",
            ));
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn numerator(self) -> i64 {
        self.numerator
    }

    pub fn denominator(self) -> u64 {
        self.denominator
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(u32);

impl StreamId {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    pub fn value(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MediaTimestamp {
    value: i64,
    timescale: u32,
}

impl MediaTimestamp {
    pub fn new(value: i64, timescale: u32) -> Result<Self> {
        if timescale == 0 {
            return Err(EngineError::invalid_argument(
                "media timestamp timescale must be greater than zero",
            ));
        }
        Ok(Self { value, timescale })
    }

    pub fn value(self) -> i64 {
        self.value
    }

    pub fn timescale(self) -> u32 {
        self.timescale
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MediaDuration {
    value: u64,
    timescale: u32,
}

impl MediaDuration {
    pub fn new(value: u64, timescale: u32) -> Result<Self> {
        if timescale == 0 {
            return Err(EngineError::invalid_argument(
                "media duration timescale must be greater than zero",
            ));
        }
        Ok(Self { value, timescale })
    }

    pub fn value(self) -> u64 {
        self.value
    }

    pub fn timescale(self) -> u32 {
        self.timescale
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MediaBuffer {
    bytes: Vec<u8>,
}

impl MediaBuffer {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MediaPacket {
    stream_id: StreamId,
    timestamp: Option<MediaTimestamp>,
    buffer: MediaBuffer,
}

impl MediaPacket {
    pub fn new(
        stream_id: StreamId,
        timestamp: Option<MediaTimestamp>,
        buffer: MediaBuffer,
    ) -> Self {
        Self {
            stream_id,
            timestamp,
            buffer,
        }
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn timestamp(&self) -> Option<MediaTimestamp> {
        self.timestamp
    }

    pub fn buffer(&self) -> &MediaBuffer {
        &self.buffer
    }

    pub fn into_buffer(self) -> MediaBuffer {
        self.buffer
    }
}

macro_rules! define_frame {
    ($name:ident) => {
        #[derive(Debug, PartialEq, Eq)]
        pub struct $name {
            stream_id: StreamId,
            timestamp: Option<MediaTimestamp>,
            buffer: MediaBuffer,
        }

        impl $name {
            pub fn new(
                stream_id: StreamId,
                timestamp: Option<MediaTimestamp>,
                buffer: MediaBuffer,
            ) -> Self {
                Self {
                    stream_id,
                    timestamp,
                    buffer,
                }
            }

            pub fn stream_id(&self) -> StreamId {
                self.stream_id
            }

            pub fn timestamp(&self) -> Option<MediaTimestamp> {
                self.timestamp
            }

            pub fn buffer(&self) -> &MediaBuffer {
                &self.buffer
            }

            pub fn into_buffer(self) -> MediaBuffer {
                self.buffer
            }
        }
    };
}

define_frame!(VideoFrame);
define_frame!(AudioFrame);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_time_rejects_zero_timescale() {
        assert!(MediaTimestamp::new(0, 0).is_err());
        assert!(MediaDuration::new(0, 0).is_err());
        assert!(Rational::new(1, 0).is_err());
    }
}
