// Copyright (C) 2021 Scott Lamb <slamb@slamb.org>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Codec-specific logic (for audio, video, and application media types).
//!
//! Currently this primarily consists of RTP depacketization logic for each
//! codec, as needed for a client during `PLAY` and a server during `RECORD`.
//! Packetization (needed for the reverse) may be added in the future.

use std::num::{NonZeroU16, NonZeroU32};

use crate::client::rtp;
use crate::ConnectionContext;
use crate::Error;
use bytes::{Buf, Bytes};
use pretty_hex::PrettyHex;

pub(crate) mod aac;
pub(crate) mod g723;

#[doc(hidden)]
pub mod h264;

pub(crate) mod onvif;
pub(crate) mod simple_audio;

#[derive(Debug)]
pub enum CodecItem {
    VideoFrame(VideoFrame),
    AudioFrame(AudioFrame),
    MessageFrame(MessageFrame),
    SenderReport(crate::client::rtp::SenderReport),
}

#[derive(Clone, Debug)]
pub enum Parameters {
    Video(VideoParameters),
    Audio(AudioParameters),
    Message(MessageParameters),
}

#[derive(Clone)]
pub struct VideoParameters {
    pixel_dimensions: (u32, u32),
    rfc6381_codec: String,
    pixel_aspect_ratio: Option<(u32, u32)>,
    frame_rate: Option<(u32, u32)>,
    extra_data: Bytes,
}

impl VideoParameters {
    /// Returns a codec description in
    /// [RFC-6381](https://tools.ietf.org/html/rfc6381) form, eg `avc1.4D401E`.
    // TODO: use https://github.com/dholroyd/rfc6381-codec crate once published?
    pub fn rfc6381_codec(&self) -> &str {
        &self.rfc6381_codec
    }

    /// Returns the overall dimensions of the video frame in pixels, as `(width, height)`.
    pub fn pixel_dimensions(&self) -> (u32, u32) {
        self.pixel_dimensions
    }

    /// Returns the displayed size of a pixel, if known, as a dimensionless ratio `(h_spacing, v_spacing)`.
    /// This is as specified in [ISO/IEC 14496-12:2015](https://standards.iso.org/ittf/PubliclyAvailableStandards/c068960_ISO_IEC_14496-12_2015.zip])
    /// section 12.1.4.
    ///
    /// It's common for IP cameras to use [anamorphic](https://en.wikipedia.org/wiki/Anamorphic_format) sub streams.
    /// Eg a 16x9 camera may export the same video source as a 1920x1080 "main"
    /// stream and a 704x480 "sub" stream, without cropping. The former has a
    /// pixel aspect ratio of `(1, 1)` while the latter has a pixel aspect ratio
    /// of `(40, 33)`.
    pub fn pixel_aspect_ratio(&self) -> Option<(u32, u32)> {
        self.pixel_aspect_ratio
    }

    /// Returns the maximum frame rate in seconds as `(numerator, denominator)`,
    /// if known.
    ///
    /// May not be minimized, and may not be in terms of the clock rate. Eg 15
    /// frames per second might be returned as `(1, 15)` or `(6000, 90000)`. The
    /// standard NTSC framerate (roughly 29.97 fps) might be returned as
    /// `(1001, 30000)`.
    ///
    /// TODO: maybe return in clock rate units instead?
    /// TODO: expose fixed vs max distinction (see H.264 fixed_frame_rate_flag).
    pub fn frame_rate(&self) -> Option<(u32, u32)> {
        self.frame_rate
    }

    /// The codec-specific "extra data" to feed to eg ffmpeg to decode the video frames.
    /// *   H.264: an AvcDecoderConfig.
    pub fn extra_data(&self) -> &Bytes {
        &self.extra_data
    }
}

impl std::fmt::Debug for VideoParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoParameters")
            .field("rfc6381_codec", &self.rfc6381_codec)
            .field("pixel_dimensions", &self.pixel_dimensions)
            .field("pixel_aspect_ratio", &self.pixel_aspect_ratio)
            .field("frame_rate", &self.frame_rate)
            .field("extra_data", &self.extra_data.hex_dump())
            .finish()
    }
}

#[derive(Clone)]
pub struct AudioParameters {
    rfc6381_codec: Option<String>,
    frame_length: Option<NonZeroU32>,
    clock_rate: u32,
    extra_data: Bytes,
    sample_entry: Option<Bytes>,
}

impl std::fmt::Debug for AudioParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioParameters")
            .field("rfc6381_codec", &self.rfc6381_codec)
            .field("frame_length", &self.frame_length)
            .field("extra_data", &self.extra_data.hex_dump())
            .finish()
    }
}

impl AudioParameters {
    pub fn rfc6381_codec(&self) -> Option<&str> {
        self.rfc6381_codec.as_deref()
    }

    /// The length of each frame (in clock_rate units), if fixed.
    pub fn frame_length(&self) -> Option<NonZeroU32> {
        self.frame_length
    }

    pub fn clock_rate(&self) -> u32 {
        self.clock_rate
    }

    /// The codec-specific "extra data" to feed to eg ffmpeg to decode the audio.
    /// *   AAC: a serialized `AudioSpecificConfig`.
    pub fn extra_data(&self) -> &Bytes {
        &self.extra_data
    }

    /// An `.mp4` `SimpleAudioEntry` box (as defined in ISO/IEC 14496-12), if possible.
    ///
    /// Not all codecs can be placed into a `.mp4` file, and even for supported codecs there
    /// may be unsupported edge cases.
    pub fn sample_entry(&self) -> Option<&Bytes> {
        self.sample_entry.as_ref()
    }
}

/// An audio frame, which consists of one or more samples.
pub struct AudioFrame {
    pub ctx: crate::RtspMessageContext,
    pub stream_id: usize,
    pub timestamp: crate::Timestamp,
    pub frame_length: NonZeroU32,

    /// Number of lost RTP packets before this audio frame. See [crate::client::rtp::Packet::loss].
    /// Note that if loss occurs during a fragmented frame, more than this number of packets' worth
    /// of data may be skipped.
    pub loss: u16,

    // TODO: expose bytes or Buf (for zero-copy)?
    pub data: Bytes,
}

impl std::fmt::Debug for AudioFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioFrame")
            .field("stream_id", &self.stream_id)
            .field("ctx", &self.ctx)
            .field("loss", &self.loss)
            .field("timestamp", &self.timestamp)
            .field("frame_length", &self.frame_length)
            .field("data", &self.data.hex_dump())
            .finish()
    }
}

impl Buf for AudioFrame {
    fn remaining(&self) -> usize {
        self.data.remaining()
    }

    fn chunk(&self) -> &[u8] {
        self.data.chunk()
    }

    fn advance(&mut self, cnt: usize) {
        self.data.advance(cnt)
    }
}

#[derive(Clone, Debug)]
pub struct MessageParameters(onvif::CompressionType);

pub struct MessageFrame {
    pub ctx: crate::RtspMessageContext,
    pub timestamp: crate::Timestamp,
    pub stream_id: usize,

    /// Number of lost RTP packets before this message frame. See [crate::client::rtp::Packet::loss].
    /// If this is non-zero, a prefix of the message may be missing.
    pub loss: u16,

    // TODO: expose bytes or Buf (for zero-copy)?
    pub data: Bytes,
}

impl std::fmt::Debug for MessageFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioFrame")
            .field("ctx", &self.ctx)
            .field("stream_id", &self.stream_id)
            .field("loss", &self.loss)
            .field("timestamp", &self.timestamp)
            .field("data", &self.data.hex_dump())
            .finish()
    }
}

/// A single encoded video frame (aka picture, video sample, or video access unit).
///
/// Use the [bytes::Buf] implementation to retrieve data. Durations aren't
/// specified here; they can be calculated from the timestamp of a following
/// picture, or approximated via the frame rate.
pub struct VideoFrame {
    // New video parameters. Rarely populated and large, so boxed to reduce bloat.
    pub new_parameters: Option<Box<VideoParameters>>,

    /// Number of lost RTP packets before this video frame. See [crate::client::rtp::Packet::loss].
    /// Note that if loss occurs during a fragmented frame, more than this number of packets' worth
    /// of data may be skipped.
    pub loss: u16,

    // A pair of contexts: for the start and for the end.
    // Having both can be useful to measure the total time elapsed while receiving the frame.
    start_ctx: crate::RtspMessageContext,
    end_ctx: crate::RtspMessageContext,

    /// This picture's timestamp in the time base associated with the stream.
    pub timestamp: crate::Timestamp,

    pub stream_id: usize,

    /// If this is a "random access point (RAP)" aka "instantaneous decoding refresh (IDR)" picture.
    /// The former is defined in ISO/IEC 14496-12; the latter in H.264. Both mean that this picture
    /// can be decoded without any other AND no pictures following this one depend on any pictures
    /// before this one.
    pub is_random_access_point: bool,

    /// If no other pictures require this one to be decoded correctly.
    /// In H.264 terms, this is a frame with `nal_ref_idc == 0`.
    pub is_disposable: bool,

    data: bytes::Bytes,
}

impl VideoFrame {
    #[inline]
    pub fn start_ctx(&self) -> crate::RtspMessageContext {
        self.start_ctx
    }

    #[inline]
    pub fn end_ctx(&self) -> crate::RtspMessageContext {
        self.end_ctx
    }

    #[inline]
    pub fn data(&self) -> &Bytes {
        &self.data
    }

    #[inline]
    pub fn into_data(self) -> Bytes {
        self.data
    }
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        //use pretty_hex::PrettyHex;
        f.debug_struct("VideoFrame")
            .field("timestamp", &self.timestamp)
            .field("start_ctx", &self.start_ctx)
            .field("end_ctx", &self.end_ctx)
            .field("loss", &self.loss)
            .field("new_parameters", &self.new_parameters)
            .field("is_random_access_point", &self.is_random_access_point)
            .field("is_disposable", &self.is_disposable)
            .field("data_len", &self.data.len())
            //.field("data", &self.data.hex_dump())
            .finish()
    }
}

/// Turns RTP packets into [CodecItem]s.
/// This interface unstable and for internal use; it's exposed for direct fuzzing and benchmarking.
#[doc(hidden)]
#[derive(Debug)]
pub struct Depacketizer(DepacketizerInner);

#[derive(Debug)]
enum DepacketizerInner {
    Aac(Box<aac::Depacketizer>),
    SimpleAudio(Box<simple_audio::Depacketizer>),
    G723(Box<g723::Depacketizer>),
    H264(Box<h264::Depacketizer>),
    Onvif(Box<onvif::Depacketizer>),
}

impl Depacketizer {
    pub fn new(
        media: &str,
        encoding_name: &str,
        clock_rate: u32,
        channels: Option<NonZeroU16>,
        format_specific_params: Option<&str>,
    ) -> Result<Self, String> {
        use onvif::CompressionType;

        // RTP Payload Format Media Types
        // https://www.iana.org/assignments/rtp-parameters/rtp-parameters.xhtml#rtp-parameters-2
        Ok(Depacketizer(match (media, encoding_name) {
            ("video", "h264") => DepacketizerInner::H264(Box::new(h264::Depacketizer::new(
                clock_rate,
                format_specific_params,
            )?)),
            ("audio", "mpeg4-generic") => DepacketizerInner::Aac(Box::new(aac::Depacketizer::new(
                clock_rate,
                channels,
                format_specific_params,
            )?)),
            ("audio", "g726-16") => DepacketizerInner::SimpleAudio(Box::new(
                simple_audio::Depacketizer::new(clock_rate, 2),
            )),
            ("audio", "g726-24") => DepacketizerInner::SimpleAudio(Box::new(
                simple_audio::Depacketizer::new(clock_rate, 3),
            )),
            ("audio", "dvi4") | ("audio", "g726-32") => DepacketizerInner::SimpleAudio(Box::new(
                simple_audio::Depacketizer::new(clock_rate, 4),
            )),
            ("audio", "g726-40") => DepacketizerInner::SimpleAudio(Box::new(
                simple_audio::Depacketizer::new(clock_rate, 5),
            )),
            ("audio", "pcma") | ("audio", "pcmu") | ("audio", "u8") | ("audio", "g722") => {
                DepacketizerInner::SimpleAudio(Box::new(simple_audio::Depacketizer::new(
                    clock_rate, 8,
                )))
            }
            ("audio", "l16") => DepacketizerInner::SimpleAudio(Box::new(
                simple_audio::Depacketizer::new(clock_rate, 16),
            )),
            // Dahua cameras when configured with G723 send packets with a
            // non-standard encoding-name "G723.1" and length 40, which doesn't
            // make sense. Don't try to depacketize these.
            ("audio", "g723") => {
                DepacketizerInner::G723(Box::new(g723::Depacketizer::new(clock_rate)?))
            }
            ("application", "vnd.onvif.metadata") => DepacketizerInner::Onvif(Box::new(
                onvif::Depacketizer::new(CompressionType::Uncompressed),
            )),
            ("application", "vnd.onvif.metadata.gzip") => DepacketizerInner::Onvif(Box::new(
                onvif::Depacketizer::new(CompressionType::GzipCompressed),
            )),
            ("application", "vnd.onvif.metadata.exi.onvif") => DepacketizerInner::Onvif(Box::new(
                onvif::Depacketizer::new(CompressionType::ExiDefault),
            )),
            ("application", "vnd.onvif.metadata.exi.ext") => DepacketizerInner::Onvif(Box::new(
                onvif::Depacketizer::new(CompressionType::ExiInBand),
            )),
            (_, _) => {
                log::info!(
                    "no depacketizer for media/encoding_name {}/{}",
                    media,
                    encoding_name
                );
                return Err(format!(
                    "no depacketizer for media/encoding_name {}/{}",
                    media, encoding_name
                ));
            }
        }))
    }

    pub fn parameters(&self) -> Option<Parameters> {
        match &self.0 {
            DepacketizerInner::Aac(d) => d.parameters(),
            DepacketizerInner::G723(d) => d.parameters(),
            DepacketizerInner::H264(d) => d.parameters(),
            DepacketizerInner::Onvif(d) => d.parameters(),
            DepacketizerInner::SimpleAudio(d) => d.parameters(),
        }
    }

    pub fn push(&mut self, input: rtp::Packet) -> Result<(), String> {
        match &mut self.0 {
            DepacketizerInner::Aac(d) => d.push(input),
            DepacketizerInner::G723(d) => d.push(input),
            DepacketizerInner::H264(d) => d.push(input),
            DepacketizerInner::Onvif(d) => d.push(input),
            DepacketizerInner::SimpleAudio(d) => d.push(input),
        }
    }

    pub fn pull(&mut self, conn_ctx: &ConnectionContext) -> Result<Option<CodecItem>, Error> {
        match &mut self.0 {
            DepacketizerInner::Aac(d) => d.pull(conn_ctx),
            DepacketizerInner::G723(d) => Ok(d.pull()),
            DepacketizerInner::H264(d) => Ok(d.pull()),
            DepacketizerInner::Onvif(d) => Ok(d.pull()),
            DepacketizerInner::SimpleAudio(d) => Ok(d.pull()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // See with: cargo test -- --nocapture codec::tests::print_sizes
    #[test]
    fn print_sizes() {
        for (name, size) in &[
            ("Depacketizer", std::mem::size_of::<Depacketizer>()),
            (
                "aac::Depacketizer",
                std::mem::size_of::<aac::Depacketizer>(),
            ),
            (
                "g723::Depacketizer",
                std::mem::size_of::<g723::Depacketizer>(),
            ),
            (
                "h264::Depacketizer",
                std::mem::size_of::<h264::Depacketizer>(),
            ),
            (
                "onvif::Depacketizer",
                std::mem::size_of::<onvif::Depacketizer>(),
            ),
            (
                "simple_audio::Depacketizer",
                std::mem::size_of::<simple_audio::Depacketizer>(),
            ),
            ("CodecItem", std::mem::size_of::<CodecItem>()),
            ("VideoFrame", std::mem::size_of::<VideoFrame>()),
            ("AudioFrame", std::mem::size_of::<AudioFrame>()),
            ("MessageFrame", std::mem::size_of::<MessageFrame>()),
            (
                "SenderReport",
                std::mem::size_of::<crate::client::rtp::SenderReport>(),
            ),
            ("Parameters", std::mem::size_of::<Parameters>()),
            ("VideoParameters", std::mem::size_of::<VideoParameters>()),
            ("AudioParameters", std::mem::size_of::<AudioParameters>()),
            (
                "MessageParameters",
                std::mem::size_of::<MessageParameters>(),
            ),
        ] {
            println!("{:-40} {:4}", name, size);
        }
    }
}
