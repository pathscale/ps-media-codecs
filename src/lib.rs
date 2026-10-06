//! Bounded adapters around the published pure-Rust `rusty-opus` and
//! `rusty_vp9` implementations.
//!
//! This crate deliberately starts with a narrow real-time profile: 48 kHz
//! Opus, and VP9 profile 0 (8-bit 4:2:0) up to 1920x1080. The adapters reject
//! oversized or unsupported inputs before passing them to the codec crates.
//! They do not establish browser interoperability or VP9 bitstream
//! conformance; those require independent known-vector and browser tests.

use std::fmt;

use rusty_opus::{Application, OpusDecoder as RustyOpusDecoder, OpusEncoder as RustyOpusEncoder};
use rusty_vp9::{
    parse_uncompressed_header, BitReader, DecodedFrame as RustyDecodedFrame,
    Error as RustyVp9Error, Vp9Decoder as RustyVp9Decoder, Vp9Encoder as RustyVp9Encoder,
    Vp9EncoderConfig,
};

/// Opus uses a fixed 20 ms frame at 48 kHz in this starter API.
pub const OPUS_SAMPLE_RATE: i32 = 48_000;
/// Samples per channel in one 20 ms Opus frame at 48 kHz.
pub const OPUS_FRAME_SAMPLES: usize = 960;
/// Maximum packet capacity accepted by these Opus adapters.
pub const MAX_OPUS_PACKET_BYTES: usize = 4 * 1024;

/// Maximum input frame dimensions accepted by the VP9 adapters.
pub const MAX_VIDEO_WIDTH: u32 = 1920;
/// Maximum input frame dimensions accepted by the VP9 adapters.
pub const MAX_VIDEO_HEIGHT: u32 = 1080;
/// Maximum coded VP9 packet size accepted by these adapters.
pub const MAX_VIDEO_PACKET_BYTES: usize = 4 * 1024 * 1024;
/// Maximum coded frames in one VP9 packet, including a superframe.
pub const MAX_VIDEO_FRAMES_PER_PACKET: usize = 2;
const MAX_VIDEO_PIXELS: u64 = MAX_VIDEO_WIDTH as u64 * MAX_VIDEO_HEIGHT as u64;

/// Errors reported by the bounded adapters.
#[derive(Debug)]
pub enum CodecError {
    /// The caller supplied an unsupported or over-limit frame or packet.
    InvalidInput(&'static str),
    /// The Opus implementation rejected a frame or packet.
    Opus(rusty_opus::Error),
    /// The VP9 implementation rejected a frame or packet.
    Vp9(RustyVp9Error),
    /// A previous VP9 operation failed after mutating decoder/encoder state.
    /// Call `reset` before reusing that codec instance.
    CodecStatePoisoned,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(f, "invalid codec input: {message}"),
            Self::Opus(error) => write!(f, "Opus: {error}"),
            Self::Vp9(error) => write!(f, "VP9: {error}"),
            Self::CodecStatePoisoned => f.write_str("codec state is poisoned; reset it first"),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<rusty_opus::Error> for CodecError {
    fn from(error: rusty_opus::Error) -> Self {
        Self::Opus(error)
    }
}

impl From<RustyVp9Error> for CodecError {
    fn from(error: RustyVp9Error) -> Self {
        Self::Vp9(error)
    }
}

/// Encode/decode one interleaved Opus frame at 48 kHz.
///
/// The channel count is fixed to mono or stereo for the lifetime of the
/// instance. Every call handles exactly 20 ms, which bounds PCM and packet
/// buffers and avoids variable-duration buffering in this initial adapter.
pub struct OpusCodec {
    encoder: RustyOpusEncoder,
    decoder: RustyOpusDecoder,
    channels: usize,
}

impl OpusCodec {
    /// Construct the 48 kHz interactive-voice codec for one or two channels.
    pub fn new(channels: usize) -> Result<Self, CodecError> {
        if !matches!(channels, 1 | 2) {
            return Err(CodecError::InvalidInput(
                "Opus supports 1 or 2 channels here",
            ));
        }

        let mut encoder = RustyOpusEncoder::new(OPUS_SAMPLE_RATE, channels, Application::Voip)?;
        encoder.bitrate_bps = if channels == 1 { 32_000 } else { 64_000 };

        Ok(Self {
            encoder,
            decoder: RustyOpusDecoder::new(OPUS_SAMPLE_RATE, channels)?,
            channels,
        })
    }

    /// Encode exactly one 20 ms interleaved PCM frame into a caller-owned
    /// packet buffer. The output slice is limited to 4 KiB.
    pub fn encode_20ms(&mut self, pcm: &[f32], output: &mut [u8]) -> Result<usize, CodecError> {
        let expected = OPUS_FRAME_SAMPLES * self.channels;
        if pcm.len() != expected {
            return Err(CodecError::InvalidInput(
                "PCM frame must contain exactly 20 ms",
            ));
        }
        if pcm.iter().any(|sample| !sample.is_finite()) {
            return Err(CodecError::InvalidInput("PCM samples must be finite"));
        }
        let output_len = output.len().min(MAX_OPUS_PACKET_BYTES);
        if output_len < 2 {
            return Err(CodecError::InvalidInput("Opus output buffer is too small"));
        }
        Ok(self
            .encoder
            .encode(pcm, OPUS_FRAME_SAMPLES, &mut output[..output_len])?)
    }

    /// Decode one Opus packet (or an empty packet for 20 ms packet-loss
    /// concealment), with 20 ms as the maximum output duration.
    pub fn decode_20ms(&mut self, packet: &[u8], output: &mut [f32]) -> Result<usize, CodecError> {
        if packet.len() > MAX_OPUS_PACKET_BYTES {
            return Err(CodecError::InvalidInput("Opus packet exceeds 4 KiB"));
        }
        if output.len() != OPUS_FRAME_SAMPLES * self.channels {
            return Err(CodecError::InvalidInput(
                "PCM output must hold exactly 20 ms",
            ));
        }
        Ok(self.decoder.decode(packet, OPUS_FRAME_SAMPLES, output)?)
    }
}

/// A borrowed 8-bit planar YUV 4:2:0 frame for the VP9 encoder.
pub struct Yuv420Frame<'a> {
    /// Visible luma width.
    pub width: u32,
    /// Visible luma height.
    pub height: u32,
    /// Y, U, and V plane slices. Chroma dimensions round up for odd sizes.
    pub planes: [&'a [u8]; 3],
    /// Y, U, and V row strides in bytes.
    pub strides: [usize; 3],
}

/// One owned decoded 8-bit YUV 4:2:0 picture.
pub struct DecodedYuv420 {
    /// Visible luma width.
    pub width: u32,
    /// Visible luma height.
    pub height: u32,
    /// Y, U, and V planes in tightly packed row order.
    pub planes: [Vec<u8>; 3],
    /// Presentation timestamp supplied to [`Vp9Decoder::decode_packet`].
    pub pts: Option<i64>,
}

/// Low-latency VP9 profile-0 encoder for 8-bit planar YUV 4:2:0.
pub struct Vp9Encoder {
    inner: RustyVp9Encoder,
    poisoned: bool,
}

impl Vp9Encoder {
    /// Create an encoder with no lookahead/two-pass buffering and a bounded
    /// realtime-oriented preset.
    pub fn new() -> Result<Self, CodecError> {
        let mut inner = RustyVp9Encoder::default();
        inner.configure(&Vp9EncoderConfig {
            qindex: Some(96),
            lag: Some(0),
            speed: Some(4),
            two_pass: false,
            ..Vp9EncoderConfig::default()
        })?;
        Ok(Self {
            inner,
            poisoned: false,
        })
    }

    /// Encode one bounded YUV 4:2:0 frame and return its coded VP9 packet.
    pub fn encode_frame(&mut self, frame: Yuv420Frame<'_>) -> Result<Vec<u8>, CodecError> {
        if self.poisoned {
            return Err(CodecError::CodecStatePoisoned);
        }
        validate_dimensions(frame.width, frame.height)?;
        let chroma_width = frame.width.div_ceil(2) as usize;
        let chroma_height = frame.height.div_ceil(2) as usize;
        validate_plane(
            frame.planes[0],
            frame.strides[0],
            frame.width as usize,
            frame.height as usize,
        )?;
        validate_plane(
            frame.planes[1],
            frame.strides[1],
            chroma_width,
            chroma_height,
        )?;
        validate_plane(
            frame.planes[2],
            frame.strides[2],
            chroma_width,
            chroma_height,
        )?;

        if let Err(error) =
            self.inner
                .push_frame(frame.planes, frame.strides, frame.width, frame.height)
        {
            self.poisoned = true;
            return Err(error.into());
        }

        match self.inner.next_packet() {
            Ok(packet) if packet.data.len() <= MAX_VIDEO_PACKET_BYTES => Ok(packet.data),
            Ok(_) => {
                self.poisoned = true;
                Err(CodecError::InvalidInput("encoded VP9 packet exceeds 4 MiB"))
            }
            Err(RustyVp9Error::Again) => {
                self.poisoned = true;
                Err(CodecError::InvalidInput(
                    "low-latency encoder did not emit a frame",
                ))
            }
            Err(error) => {
                self.poisoned = true;
                Err(error.into())
            }
        }
    }

    /// Recreate the encoder after a late internal error.
    pub fn reset(&mut self) -> Result<(), CodecError> {
        *self = Self::new()?;
        Ok(())
    }
}

/// Stateful bounded VP9 profile-0 decoder.
pub struct Vp9Decoder {
    inner: RustyVp9Decoder,
    poisoned: bool,
}

impl Vp9Decoder {
    /// Construct an empty decoder.
    pub fn new() -> Self {
        Self {
            inner: RustyVp9Decoder::new(),
            poisoned: false,
        }
    }

    /// Validate and decode one VP9 frame or superframe packet. Each packet is
    /// limited to 4 MiB and at most two coded frames; declared dimensions are
    /// checked before the upstream decoder can allocate frame planes.
    pub fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: Option<i64>,
    ) -> Result<Vec<DecodedYuv420>, CodecError> {
        if self.poisoned {
            return Err(CodecError::CodecStatePoisoned);
        }
        if packet.is_empty() || packet.len() > MAX_VIDEO_PACKET_BYTES {
            return Err(CodecError::InvalidInput(
                "VP9 packet must be between 1 byte and 4 MiB",
            ));
        }

        let frames = split_superframe(packet)?;
        if frames.len() > MAX_VIDEO_FRAMES_PER_PACKET {
            return Err(CodecError::InvalidInput(
                "VP9 superframe has more than two frames",
            ));
        }
        for frame in &frames {
            preflight_vp9_frame(frame)?;
        }

        if let Err(error) = self.inner.push(packet, pts) {
            self.poisoned = true;
            return Err(error.into());
        }

        let mut output = Vec::with_capacity(frames.len());
        for _ in 0..frames.len() {
            match self.inner.next_frame() {
                Ok(frame) => match convert_frame(frame) {
                    Ok(frame) => output.push(frame),
                    Err(error) => {
                        self.poisoned = true;
                        return Err(error);
                    }
                },
                // Hidden ALT-REF frames and lower spatial layers are consumed
                // without display by the upstream decoder.
                Err(RustyVp9Error::Again) => {}
                Err(error) => {
                    self.poisoned = true;
                    return Err(error.into());
                }
            }
        }
        Ok(output)
    }

    /// Reset references and error state after malformed or unsupported data.
    pub fn reset(&mut self) {
        self.inner = RustyVp9Decoder::new();
        self.poisoned = false;
    }
}

impl Default for Vp9Decoder {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), CodecError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_VIDEO_WIDTH
        || height > MAX_VIDEO_HEIGHT
        || pixels > MAX_VIDEO_PIXELS
    {
        return Err(CodecError::InvalidInput(
            "video dimensions exceed 1920x1080",
        ));
    }
    Ok(())
}

fn validate_plane(
    plane: &[u8],
    stride: usize,
    row_bytes: usize,
    rows: usize,
) -> Result<(), CodecError> {
    if row_bytes == 0 || rows == 0 || stride < row_bytes || stride > 8192 {
        return Err(CodecError::InvalidInput(
            "invalid YUV plane stride or dimensions",
        ));
    }
    let required = (rows - 1)
        .checked_mul(stride)
        .and_then(|prefix| prefix.checked_add(row_bytes))
        .ok_or(CodecError::InvalidInput("YUV plane length overflow"))?;
    if plane.len() < required {
        return Err(CodecError::InvalidInput(
            "YUV plane slice is shorter than its stride",
        ));
    }
    Ok(())
}

fn split_superframe(packet: &[u8]) -> Result<Vec<&[u8]>, CodecError> {
    let Some(&marker) = packet.last() else {
        return Err(CodecError::InvalidInput("empty VP9 packet"));
    };
    if marker & 0xe0 != 0xc0 {
        return Ok(vec![packet]);
    }

    let frame_count = usize::from(marker & 0x07) + 1;
    let magnitude = usize::from((marker >> 3) & 0x03) + 1;
    let index_size = 2usize
        .checked_add(
            magnitude
                .checked_mul(frame_count)
                .ok_or(CodecError::InvalidInput("VP9 superframe index overflow"))?,
        )
        .ok_or(CodecError::InvalidInput("VP9 superframe index overflow"))?;
    if packet.len() < index_size {
        return Ok(vec![packet]);
    }
    let index_start = packet.len() - index_size;
    if packet[index_start] != marker {
        return Ok(vec![packet]);
    }

    let mut offset = 0usize;
    let mut index = index_start + 1;
    let mut frames = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let mut frame_size = 0usize;
        for byte_index in 0..magnitude {
            let byte = usize::from(packet[index]);
            index += 1;
            frame_size |= byte << (byte_index * 8);
        }
        if frame_size == 0 {
            return Err(CodecError::InvalidInput("zero-sized VP9 superframe member"));
        }
        let end = offset
            .checked_add(frame_size)
            .ok_or(CodecError::InvalidInput("VP9 frame size overflow"))?;
        if end > index_start {
            return Err(CodecError::InvalidInput(
                "VP9 superframe index overlaps frame data",
            ));
        }
        frames.push(&packet[offset..end]);
        offset = end;
    }
    if offset != index_start || index != packet.len() - 1 {
        return Err(CodecError::InvalidInput(
            "VP9 superframe index does not cover packet",
        ));
    }
    Ok(frames)
}

fn preflight_vp9_frame(frame: &[u8]) -> Result<(), CodecError> {
    if frame.is_empty() {
        return Err(CodecError::InvalidInput("empty VP9 frame"));
    }
    let mut bits = BitReader::new(frame);
    // Previous references have already passed this adapter's dimension cap.
    // Using the maximum for every slot ensures a size inherited from any
    // reference is conservatively checked before decode allocation.
    let max_refs = [(MAX_VIDEO_WIDTH, MAX_VIDEO_HEIGHT); 8];
    let header = parse_uncompressed_header(&mut bits, &max_refs)?;
    if header.profile != 0 {
        return Err(CodecError::InvalidInput("only VP9 profile 0 is supported"));
    }
    if header.show_existing_frame {
        return Ok(());
    }
    if !header.sized {
        return Err(CodecError::InvalidInput(
            "VP9 frame header has no resolved dimensions",
        ));
    }
    validate_dimensions(header.width, header.height)?;
    if header.key_frame || header.intra_only {
        if header.bit_depth != 8 || header.subsampling_x != 1 || header.subsampling_y != 1 {
            return Err(CodecError::InvalidInput(
                "only 8-bit 4:2:0 VP9 is supported",
            ));
        }
    }
    Ok(())
}

fn convert_frame(frame: RustyDecodedFrame) -> Result<DecodedYuv420, CodecError> {
    validate_dimensions(frame.width, frame.height)?;
    if frame.bit_depth != 8
        || frame.subsampling_x != 1
        || frame.subsampling_y != 1
        || frame.planes.len() != 3
        || frame.strides.len() != 3
    {
        return Err(CodecError::InvalidInput(
            "decoded VP9 frame is not 8-bit 4:2:0",
        ));
    }

    let widths = [
        frame.width as usize,
        frame.width.div_ceil(2) as usize,
        frame.width.div_ceil(2) as usize,
    ];
    let heights = [
        frame.height as usize,
        frame.height.div_ceil(2) as usize,
        frame.height.div_ceil(2) as usize,
    ];
    let mut planes = frame.planes.into_iter();
    let mut checked = Vec::with_capacity(3);
    for index in 0..3 {
        let plane = planes
            .next()
            .ok_or(CodecError::InvalidInput("missing decoded VP9 plane"))?;
        let expected =
            widths[index]
                .checked_mul(heights[index])
                .ok_or(CodecError::InvalidInput(
                    "decoded VP9 plane length overflow",
                ))?;
        if frame.strides[index] != widths[index] || plane.len() != expected {
            return Err(CodecError::InvalidInput(
                "decoded VP9 plane layout is unexpected",
            ));
        }
        checked.push(plane);
    }
    let planes: [Vec<u8>; 3] = checked
        .try_into()
        .map_err(|_| CodecError::InvalidInput("decoded VP9 plane count changed"))?;

    Ok(DecodedYuv420 {
        width: frame.width,
        height: frame.height,
        planes,
        pts: frame.pts,
    })
}
