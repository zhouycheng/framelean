use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;

use framelean_core::{EngineError, EngineErrorCode, ErrorKind, Result};
use rusty_ffmpeg::ffi;

use super::{InputContext, Packet, media_native_error};

const MICROSECONDS_TIME_BASE: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: 1_000_000,
};
const THUMBNAIL_PROBE_SIZE: u32 = 16;
const BLACK_LUMA_THRESHOLD: f64 = 18.0;
const VISIBLE_PIXEL_LUMA_THRESHOLD: f64 = 28.0;
const VISIBLE_PIXEL_RATIO_THRESHOLD: f64 = 0.02;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedVideoFrame {
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub rgb24: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFramesRequest {
    pub input_path: PathBuf,
    pub output_directory: PathBuf,
    pub timestamps_us: Vec<u64>,
    pub max_width: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrameArtifact {
    pub index: usize,
    pub requested_timestamp_us: u64,
    pub decoded_timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFramesResult {
    pub output_directory: PathBuf,
    pub frames: Vec<PreviewFrameArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoThumbnailRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub duration_us: Option<u64>,
    pub max_width: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoThumbnailResult {
    pub output_path: PathBuf,
    pub requested_timestamp_us: u64,
    pub decoded_timestamp_us: u64,
    pub width: u32,
    pub height: u32,
}

pub(super) fn decode_video_frame(
    input_path: &Path,
    timestamp_us: u64,
    max_width: Option<u32>,
) -> Result<DecodedVideoFrame> {
    let input = InputContext::open(input_path)?;
    let decoder = VideoDecoder::open(&input)?;
    decoder.decode_at(&input, timestamp_us, max_width)
}

pub(super) fn generate_preview_frames(
    request: &PreviewFramesRequest,
) -> Result<PreviewFramesResult> {
    if request.timestamps_us.is_empty() {
        return Err(EngineError::invalid_argument(
            "preview frame timestamps must not be empty",
        ));
    }
    fs::create_dir_all(&request.output_directory).map_err(|error| {
        EngineError::with_code(
            ErrorKind::Runtime,
            EngineErrorCode::OutputContainerNotWritable,
            format!("failed to create preview directory: {error}"),
        )
    })?;

    let mut frames = Vec::with_capacity(request.timestamps_us.len());
    for (offset, timestamp_us) in request.timestamps_us.iter().copied().enumerate() {
        let frame = decode_video_frame(&request.input_path, timestamp_us, request.max_width)?;
        let output_path = request
            .output_directory
            .join(format!("frame_{}.bmp", offset + 1));
        write_bmp_transactionally(&output_path, &frame)?;
        frames.push(PreviewFrameArtifact {
            index: offset + 1,
            requested_timestamp_us: timestamp_us,
            decoded_timestamp_us: frame.timestamp_us,
            width: frame.width,
            height: frame.height,
            output_path,
        });
    }

    Ok(PreviewFramesResult {
        output_directory: request.output_directory.clone(),
        frames,
    })
}

pub(super) fn generate_video_thumbnail(
    request: &VideoThumbnailRequest,
) -> Result<VideoThumbnailResult> {
    if request.max_width == 0 {
        return Err(EngineError::invalid_argument(
            "thumbnail max_width must be greater than zero",
        ));
    }
    let candidates = thumbnail_candidate_timestamps(request.duration_us);
    for requested_timestamp_us in candidates {
        let probe = match decode_video_frame(
            &request.input_path,
            requested_timestamp_us,
            Some(THUMBNAIL_PROBE_SIZE),
        ) {
            Ok(frame) => frame,
            Err(_) => continue,
        };
        if is_black_frame(&probe.rgb24) {
            continue;
        }
        let frame = decode_video_frame(
            &request.input_path,
            requested_timestamp_us,
            Some(request.max_width),
        )?;
        write_bmp_transactionally(&request.output_path, &frame)?;
        return Ok(VideoThumbnailResult {
            output_path: request.output_path.clone(),
            requested_timestamp_us,
            decoded_timestamp_us: frame.timestamp_us,
            width: frame.width,
            height: frame.height,
        });
    }

    Err(EngineError::with_code(
        ErrorKind::Media,
        EngineErrorCode::MediaInfoReadFailed,
        "no visible non-black video frame was found for the thumbnail",
    ))
}

struct VideoDecoder {
    context: *mut ffi::AVCodecContext,
    stream_index: i32,
    time_base: ffi::AVRational,
}

impl VideoDecoder {
    fn open(input: &InputContext) -> Result<Self> {
        let mut codec = ptr::null();
        // SAFETY: input owns a valid format context; decoder is an out pointer.
        let stream_index = unsafe {
            ffi::av_find_best_stream(input.0, ffi::AVMEDIA_TYPE_VIDEO, -1, -1, &mut codec, 0)
        };
        if stream_index < 0 {
            return Err(media_native_error(
                EngineErrorCode::MediaStreamUnrecognized,
                "failed to find a decodable video stream",
                stream_index,
            ));
        }
        if codec.is_null() {
            return Err(EngineError::with_code(
                ErrorKind::Media,
                EngineErrorCode::MediaCapabilityIncompatible,
                "the selected video stream has no available decoder",
            ));
        }
        // SAFETY: stream_index is returned by av_find_best_stream for this context.
        let stream = unsafe { &**input.as_ref().streams.add(stream_index as usize) };
        // SAFETY: codec is a process-owned descriptor returned by libavcodec.
        let context = unsafe { ffi::avcodec_alloc_context3(codec) };
        if context.is_null() {
            return Err(native_allocation_error("video decoder context"));
        }
        // SAFETY: both pointers are valid and the context is exclusively owned here.
        let copy_result = unsafe { ffi::avcodec_parameters_to_context(context, stream.codecpar) };
        if copy_result < 0 {
            let mut owned = context;
            // SAFETY: context was allocated above and has not been freed.
            unsafe { ffi::avcodec_free_context(&mut owned) };
            return Err(media_native_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to initialize the video decoder context",
                copy_result,
            ));
        }
        // SAFETY: context and codec are compatible and options are omitted.
        let open_result = unsafe { ffi::avcodec_open2(context, codec, ptr::null_mut()) };
        if open_result < 0 {
            let mut owned = context;
            // SAFETY: context was allocated above and has not been freed.
            unsafe { ffi::avcodec_free_context(&mut owned) };
            return Err(media_native_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to open the video decoder",
                open_result,
            ));
        }
        Ok(Self {
            context,
            stream_index,
            time_base: stream.time_base,
        })
    }

    fn decode_at(
        &self,
        input: &InputContext,
        timestamp_us: u64,
        max_width: Option<u32>,
    ) -> Result<DecodedVideoFrame> {
        let target_timestamp = unsafe {
            ffi::av_rescale_q(
                timestamp_us.min(i64::MAX as u64) as i64,
                MICROSECONDS_TIME_BASE,
                self.time_base,
            )
        };
        // SAFETY: input and stream index are valid; seek resets demuxer position only.
        let seek_result = unsafe {
            ffi::avformat_seek_file(
                input.0,
                self.stream_index,
                i64::MIN,
                target_timestamp,
                target_timestamp,
                ffi::AVSEEK_FLAG_BACKWARD as i32,
            )
        };
        if seek_result < 0 && timestamp_us != 0 {
            return Err(media_native_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to seek to the requested preview timestamp",
                seek_result,
            ));
        }
        // SAFETY: the decoder context is open and exclusively used by this call.
        unsafe { ffi::avcodec_flush_buffers(self.context) };

        let packet = Packet::new()?;
        let frame = Frame::new()?;
        let mut decoded_frames = 0_u32;
        loop {
            // SAFETY: input and packet are valid owned pointers.
            let read_result = unsafe { ffi::av_read_frame(input.0, packet.0) };
            if read_result == ffi::AVERROR_EOF {
                if let Some(frame) =
                    self.receive_frame(&frame, target_timestamp, max_width, &mut decoded_frames)?
                {
                    return Ok(frame);
                }
                break;
            }
            if read_result < 0 {
                return Err(media_native_error(
                    EngineErrorCode::MediaInfoReadFailed,
                    "failed while reading video packets",
                    read_result,
                ));
            }
            // SAFETY: av_read_frame initialized packet.
            let belongs_to_video = unsafe { (*packet.0).stream_index == self.stream_index };
            if !belongs_to_video {
                // SAFETY: packet is allocated and initialized.
                unsafe { ffi::av_packet_unref(packet.0) };
                continue;
            }
            // SAFETY: decoder and packet are valid and belong to the selected stream.
            let send_result = unsafe { ffi::avcodec_send_packet(self.context, packet.0) };
            // SAFETY: decoder has consumed or referenced packet contents.
            unsafe { ffi::av_packet_unref(packet.0) };
            if send_result < 0 && send_result != ffi::AVERROR(ffi::EAGAIN) {
                return Err(media_native_error(
                    EngineErrorCode::MediaInfoReadFailed,
                    "failed to send a video packet to the decoder",
                    send_result,
                ));
            }
            if let Some(frame) =
                self.receive_frame(&frame, target_timestamp, max_width, &mut decoded_frames)?
            {
                return Ok(frame);
            }
            if decoded_frames > 2_000 {
                break;
            }
        }

        Err(EngineError::with_code(
            ErrorKind::Media,
            EngineErrorCode::MediaInfoReadFailed,
            "no decoded video frame was available at the requested timestamp",
        ))
    }

    fn receive_frame(
        &self,
        frame: &Frame,
        target_timestamp: i64,
        max_width: Option<u32>,
        decoded_frames: &mut u32,
    ) -> Result<Option<DecodedVideoFrame>> {
        loop {
            // SAFETY: decoder and frame are valid; frame is reused only after unref.
            let receive_result = unsafe { ffi::avcodec_receive_frame(self.context, frame.0) };
            if receive_result == ffi::AVERROR(ffi::EAGAIN) || receive_result == ffi::AVERROR_EOF {
                return Ok(None);
            }
            if receive_result < 0 {
                return Err(media_native_error(
                    EngineErrorCode::MediaInfoReadFailed,
                    "failed to receive a decoded video frame",
                    receive_result,
                ));
            }
            *decoded_frames = decoded_frames.saturating_add(1);
            // SAFETY: avcodec_receive_frame populated the frame.
            let frame_ref = unsafe { &*frame.0 };
            let frame_timestamp = frame_ref.best_effort_timestamp;
            let is_target =
                frame_timestamp == ffi::AV_NOPTS_VALUE || frame_timestamp >= target_timestamp;
            if is_target {
                let timestamp_us = if frame_timestamp == ffi::AV_NOPTS_VALUE {
                    0
                } else {
                    // SAFETY: both rationals are valid FFmpeg time bases.
                    unsafe {
                        ffi::av_rescale_q(frame_timestamp, self.time_base, MICROSECONDS_TIME_BASE)
                    }
                    .max(0) as u64
                };
                let converted = convert_to_rgb24(frame_ref, timestamp_us, max_width)?;
                // SAFETY: frame is allocated and its references may be released for reuse.
                unsafe { ffi::av_frame_unref(frame.0) };
                return Ok(Some(converted));
            }
            // SAFETY: frame is allocated and its references may be released for reuse.
            unsafe { ffi::av_frame_unref(frame.0) };
        }
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        // SAFETY: context is owned by this guard and freed exactly once.
        unsafe { ffi::avcodec_free_context(&mut self.context) };
    }
}

struct Frame(*mut ffi::AVFrame);

impl Frame {
    fn new() -> Result<Self> {
        // SAFETY: allocation has no pointer preconditions.
        let frame = unsafe { ffi::av_frame_alloc() };
        if frame.is_null() {
            return Err(native_allocation_error("decoded video frame"));
        }
        Ok(Self(frame))
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: frame is owned by this guard and freed exactly once.
        unsafe { ffi::av_frame_free(&mut self.0) };
    }
}

struct ScaleContext(*mut ffi::SwsContext);

impl Drop for ScaleContext {
    fn drop(&mut self) {
        // SAFETY: context is owned by this guard and freed exactly once.
        unsafe { ffi::sws_freeContext(self.0) };
    }
}

fn convert_to_rgb24(
    frame: &ffi::AVFrame,
    timestamp_us: u64,
    max_width: Option<u32>,
) -> Result<DecodedVideoFrame> {
    let source_width = u32::try_from(frame.width).map_err(|_| invalid_frame_dimensions())?;
    let source_height = u32::try_from(frame.height).map_err(|_| invalid_frame_dimensions())?;
    if source_width == 0 || source_height == 0 {
        return Err(invalid_frame_dimensions());
    }
    let target_width = max_width
        .filter(|width| *width > 0 && *width < source_width)
        .unwrap_or(source_width);
    let target_height = ((u64::from(source_height) * u64::from(target_width)
        + u64::from(source_width) / 2)
        / u64::from(source_width))
    .max(1) as u32;
    let byte_len = usize::try_from(target_width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(target_height).ok()?))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| EngineError::invalid_argument("decoded frame dimensions are too large"))?;
    let mut rgb24 = vec![0_u8; byte_len];
    let mut destination_data = [ptr::null_mut(); 4];
    destination_data[0] = rgb24.as_mut_ptr();
    let mut destination_linesize = [0_i32; 4];
    destination_linesize[0] = i32::try_from(target_width.saturating_mul(3))
        .map_err(|_| EngineError::invalid_argument("decoded frame row is too wide"))?;
    // SAFETY: dimensions and pixel formats are valid; filters and parameters are optional.
    let scaler = unsafe {
        ffi::sws_getContext(
            frame.width,
            frame.height,
            frame.format as ffi::AVPixelFormat,
            target_width as i32,
            target_height as i32,
            ffi::AV_PIX_FMT_RGB24,
            ffi::SWS_BILINEAR as i32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if scaler.is_null() {
        return Err(EngineError::with_code(
            ErrorKind::Media,
            EngineErrorCode::MediaPixelFormatUnavailable,
            "failed to create a pixel conversion context for the decoded frame",
        ));
    }
    let scaler = ScaleContext(scaler);
    // SAFETY: source frame data is initialized by the decoder and destination points to rgb24.
    let scaled_height = unsafe {
        ffi::sws_scale(
            scaler.0,
            frame.data.as_ptr() as *const *const u8,
            frame.linesize.as_ptr(),
            0,
            frame.height,
            destination_data.as_ptr(),
            destination_linesize.as_ptr(),
        )
    };
    if scaled_height != target_height as i32 {
        return Err(EngineError::with_code(
            ErrorKind::Media,
            EngineErrorCode::MediaInfoReadFailed,
            "pixel conversion returned an incomplete video frame",
        ));
    }
    Ok(DecodedVideoFrame {
        timestamp_us,
        width: target_width,
        height: target_height,
        rgb24,
    })
}

fn write_bmp_transactionally(path: &Path, frame: &DecodedVideoFrame) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| output_error("create output directory", error))?;
    }
    let temporary_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = bmp_bytes(frame)?;
    fs::write(&temporary_path, bytes)
        .map_err(|error| output_error("write temporary bitmap", error))?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| output_error("replace existing bitmap", error))?;
    }
    fs::rename(&temporary_path, path).map_err(|error| output_error("commit bitmap", error))?;
    Ok(())
}

fn bmp_bytes(frame: &DecodedVideoFrame) -> Result<Vec<u8>> {
    let width = usize::try_from(frame.width).map_err(|_| invalid_frame_dimensions())?;
    let height = usize::try_from(frame.height).map_err(|_| invalid_frame_dimensions())?;
    let source_row = width
        .checked_mul(3)
        .ok_or_else(|| EngineError::invalid_argument("bitmap row is too wide"))?;
    if frame.rgb24.len() != source_row.saturating_mul(height) {
        return Err(EngineError::invalid_argument(
            "RGB frame byte length does not match its dimensions",
        ));
    }
    let output_row = (source_row + 3) & !3;
    let pixel_bytes = output_row
        .checked_mul(height)
        .ok_or_else(|| EngineError::invalid_argument("bitmap is too large"))?;
    let file_size = 54_usize
        .checked_add(pixel_bytes)
        .ok_or_else(|| EngineError::invalid_argument("bitmap is too large"))?;
    let mut output = Vec::with_capacity(file_size);
    output.extend_from_slice(b"BM");
    output.extend_from_slice(&(file_size as u32).to_le_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&54_u32.to_le_bytes());
    output.extend_from_slice(&40_u32.to_le_bytes());
    output.extend_from_slice(&(frame.width as i32).to_le_bytes());
    output.extend_from_slice(&(frame.height as i32).to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&24_u16.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    output.extend_from_slice(&2835_i32.to_le_bytes());
    output.extend_from_slice(&2835_i32.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    output.extend_from_slice(&0_u32.to_le_bytes());
    let padding = [0_u8; 3];
    for row in (0..height).rev() {
        let start = row * source_row;
        for pixel in frame.rgb24[start..start + source_row].chunks_exact(3) {
            output.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        output.extend_from_slice(&padding[..output_row - source_row]);
    }
    Ok(output)
}

fn thumbnail_candidate_timestamps(duration_us: Option<u64>) -> Vec<u64> {
    let Some(duration_us) = duration_us.filter(|value| *value > 0) else {
        return vec![100_000, 500_000, 1_000_000, 2_000_000, 5_000_000];
    };
    let minimum = 100_000_u64.min(duration_us);
    let mut values = vec![
        minimum,
        duration_us.saturating_mul(5) / 100,
        duration_us.saturating_mul(12) / 100,
        duration_us.saturating_mul(25) / 100,
        duration_us.saturating_mul(50) / 100,
        duration_us.saturating_mul(75) / 100,
        duration_us.saturating_mul(90) / 100,
    ];
    for value in &mut values {
        *value = (*value).clamp(minimum, duration_us);
    }
    values.sort_unstable();
    values.dedup();
    values
}

fn is_black_frame(rgb24: &[u8]) -> bool {
    let pixel_count = rgb24.len() / 3;
    if pixel_count == 0 {
        return true;
    }
    let mut luma_total = 0.0;
    let mut visible_pixels = 0_usize;
    for pixel in rgb24.chunks_exact(3) {
        let luma = (f64::from(pixel[0]) * 0.2126)
            + (f64::from(pixel[1]) * 0.7152)
            + (f64::from(pixel[2]) * 0.0722);
        luma_total += luma;
        if luma >= VISIBLE_PIXEL_LUMA_THRESHOLD {
            visible_pixels += 1;
        }
    }
    let average_luma = luma_total / pixel_count as f64;
    let visible_ratio = visible_pixels as f64 / pixel_count as f64;
    average_luma < BLACK_LUMA_THRESHOLD && visible_ratio < VISIBLE_PIXEL_RATIO_THRESHOLD
}

fn native_allocation_error(resource: &str) -> EngineError {
    EngineError::with_code(
        ErrorKind::NativeLibrary,
        EngineErrorCode::NativeLibraryUnavailable,
        format!("failed to allocate {resource}"),
    )
}

fn invalid_frame_dimensions() -> EngineError {
    EngineError::with_code(
        ErrorKind::Media,
        EngineErrorCode::MediaInfoReadFailed,
        "decoded video frame has invalid dimensions",
    )
}

fn output_error(operation: &str, error: std::io::Error) -> EngineError {
    EngineError::with_code(
        ErrorKind::Runtime,
        EngineErrorCode::OutputContainerNotWritable,
        format!("failed to {operation}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_candidates_are_sorted_and_bounded() {
        let candidates = thumbnail_candidate_timestamps(Some(10_000_000));
        assert_eq!(candidates.first().copied(), Some(100_000));
        assert_eq!(candidates.last().copied(), Some(9_000_000));
        assert!(candidates.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn black_frame_detection_matches_thumbnail_policy() {
        assert!(is_black_frame(&vec![0; 16 * 16 * 3]));
        assert!(!is_black_frame(&vec![255; 16 * 16 * 3]));
    }

    #[test]
    fn bitmap_encoder_writes_a_valid_header_and_bottom_up_pixels() {
        let frame = DecodedVideoFrame {
            timestamp_us: 0,
            width: 1,
            height: 2,
            rgb24: vec![255, 0, 0, 0, 255, 0],
        };
        let bytes = bmp_bytes(&frame).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        assert_eq!(&bytes[54..57], &[0, 255, 0]);
        assert_eq!(&bytes[58..61], &[0, 0, 255]);
    }
}
