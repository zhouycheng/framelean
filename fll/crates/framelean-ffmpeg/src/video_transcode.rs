use std::ffi::{CStr, CString, c_void};
use std::path::PathBuf;
use std::ptr;

use framelean_core::{EngineError, EngineErrorCode, ErrorKind, Result};
use rusty_ffmpeg::ffi;

use super::{InputContext, OutputContext, Packet};

const MICROSECONDS_TIME_BASE: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: 1_000_000,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTranscodeRequest {
    pub input_stream_index: u32,
    pub decoder_name: String,
    pub encoder_name: String,
    pub target_bitrate_bps: Option<u64>,
    pub target_sample_rate_hz: Option<u32>,
    pub target_channel_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFileTranscodeRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub audio_streams: Vec<AudioTranscodeRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoTranscodeRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub input_stream_index: u32,
    pub decoder_name: String,
    pub encoder_name: String,
    pub output_pixel_format: String,
    pub output_profile: Option<String>,
    pub target_bitrate_bps: Option<u64>,
    pub audio_streams: Vec<AudioTranscodeRequest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscodeProgress {
    pub media_time_us: u64,
    pub processed_bytes: u64,
    pub decoded_frames: u64,
    pub encoded_packets: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodeControl {
    Continue,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodeOutcome {
    Completed(TranscodeProgress),
    Cancelled(TranscodeProgress),
}

pub(super) fn transcode_audio(
    request: &AudioFileTranscodeRequest,
    mut control: impl FnMut(TranscodeProgress) -> TranscodeControl,
) -> Result<TranscodeOutcome> {
    validate_audio_stream_requests(&request.audio_streams)?;
    let input = InputContext::open(&request.input_path)?;
    validate_audio_only_input(&input, &request.audio_streams)?;

    let mut output = OutputContext::create(&request.output_path)?;
    let global_header = output_requires_global_header(&output);
    let mut audio_streams = request
        .audio_streams
        .iter()
        .map(|audio| {
            let input_stream = selected_audio_stream(&input, audio.input_stream_index)?;
            AudioTranscodeState::new(audio, input_stream, &mut output, global_header)
        })
        .collect::<Result<Vec<_>>>()?;
    output.open_io(&request.output_path)?;
    output.write_header()?;

    let input_packet = Packet::new_for_execution()?;
    let mut progress = TranscodeProgress::default();
    loop {
        // SAFETY: input and packet are valid owned pointers.
        let read_result = unsafe { ffi::av_read_frame(input.0, input_packet.0) };
        if read_result == ffi::AVERROR_EOF {
            break;
        }
        if read_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed while reading audio packets",
                read_result,
            ));
        }
        // SAFETY: av_read_frame initialized the packet.
        let packet_ref = unsafe { &*input_packet.0 };
        let Some(audio) = audio_streams
            .iter_mut()
            .find(|audio| audio.input_stream_index == packet_ref.stream_index as u32)
        else {
            // SAFETY: packet is allocated and initialized.
            unsafe { ffi::av_packet_unref(input_packet.0) };
            continue;
        };
        update_input_progress(&mut progress, packet_ref, audio.input_time_base);
        if control(progress) == TranscodeControl::Cancel {
            // SAFETY: packet is allocated and initialized.
            unsafe { ffi::av_packet_unref(input_packet.0) };
            return Ok(TranscodeOutcome::Cancelled(progress));
        }
        audio.process_packet(input_packet.0, output.context, &mut progress)?;
        // SAFETY: decoder has consumed or referenced the packet contents.
        unsafe { ffi::av_packet_unref(input_packet.0) };
    }

    for audio in &mut audio_streams {
        audio.finish(output.context, &mut progress)?;
    }
    output.write_trailer()?;
    Ok(TranscodeOutcome::Completed(progress))
}

pub(super) fn transcode_video(
    request: &VideoTranscodeRequest,
    mut control: impl FnMut(TranscodeProgress) -> TranscodeControl,
) -> Result<TranscodeOutcome> {
    validate_request(request)?;
    let input = InputContext::open(&request.input_path)?;
    let input_stream = selected_video_stream(&input, request.input_stream_index)?;
    validate_selected_input(&input, request.input_stream_index, &request.audio_streams)?;

    let decoder_name = c_string(&request.decoder_name, "decoder name")?;
    // SAFETY: decoder_name is a valid, null-terminated codec name.
    let decoder = unsafe { ffi::avcodec_find_decoder_by_name(decoder_name.as_ptr()) };
    if decoder.is_null() {
        return Err(chain_not_ready(format!(
            "selected decoder {} is unavailable",
            request.decoder_name
        )));
    }
    let decoder_context = CodecContext::decoder(decoder, input_stream.codecpar)?;

    let encoder_name = c_string(&request.encoder_name, "encoder name")?;
    // SAFETY: encoder_name is a valid, null-terminated codec name.
    let encoder = unsafe { ffi::avcodec_find_encoder_by_name(encoder_name.as_ptr()) };
    if encoder.is_null() {
        return Err(chain_not_ready(format!(
            "selected encoder {} is unavailable",
            request.encoder_name
        )));
    }

    let output_pixel_format = c_string(&request.output_pixel_format, "output pixel format")?;
    // SAFETY: output_pixel_format is a valid, null-terminated pixel format name.
    let pixel_format = unsafe { ffi::av_get_pix_fmt(output_pixel_format.as_ptr()) };
    if pixel_format == ffi::AV_PIX_FMT_NONE {
        return Err(chain_not_ready(format!(
            "selected output pixel format {} is unavailable",
            request.output_pixel_format
        )));
    }
    if !codec_supports_pixel_format(encoder, pixel_format)? {
        return Err(chain_not_ready(format!(
            "encoder {} does not support pixel format {}",
            request.encoder_name, request.output_pixel_format
        )));
    }

    let mut output = OutputContext::create(&request.output_path)?;
    let frame_rate = input_frame_rate(&input, input_stream);
    let encoder_context = CodecContext::video_encoder(
        encoder,
        &decoder_context,
        pixel_format,
        frame_rate,
        request.target_bitrate_bps,
        request.output_profile.as_deref(),
        output_requires_global_header(&output),
    )?;
    let output_stream = create_output_stream(&mut output, &encoder_context, "video")?;
    let global_header = output_requires_global_header(&output);
    let mut audio_states = request
        .audio_streams
        .iter()
        .map(|audio| {
            let stream = selected_audio_stream(&input, audio.input_stream_index)?;
            AudioTranscodeState::new(audio, stream, &mut output, global_header)
        })
        .collect::<Result<Vec<_>>>()?;
    output.open_io(&request.output_path)?;
    output.write_header()?;

    let input_time_base = input_stream.time_base;
    let encoder_time_base = encoder_context.as_ref().time_base;
    let output_time_base = unsafe { (*output_stream).time_base };
    let input_packet = Packet::new_for_execution()?;
    let encoded_packet = Packet::new_for_execution()?;
    let decoded_frame = Frame::new()?;
    let mut converter = FrameConverter::default();
    let mut next_pts = 0_i64;
    let mut progress = TranscodeProgress::default();

    loop {
        // SAFETY: input and packet are valid owned pointers.
        let read_result = unsafe { ffi::av_read_frame(input.0, input_packet.0) };
        if read_result == ffi::AVERROR_EOF {
            break;
        }
        if read_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed while reading video packets",
                read_result,
            ));
        }
        // SAFETY: av_read_frame initialized the packet.
        let packet_ref = unsafe { &*input_packet.0 };
        if packet_ref.stream_index != request.input_stream_index as i32
            && !audio_states
                .iter()
                .any(|audio| packet_ref.stream_index == audio.input_stream_index as i32)
        {
            // SAFETY: packet is allocated and initialized.
            unsafe { ffi::av_packet_unref(input_packet.0) };
            continue;
        }

        let packet_time_base = if packet_ref.stream_index == request.input_stream_index as i32 {
            input_time_base
        } else {
            audio_states
                .iter()
                .find(|audio| audio.input_stream_index == packet_ref.stream_index as u32)
                .expect("selected audio packets require audio state")
                .input_time_base
        };
        update_input_progress(&mut progress, packet_ref, packet_time_base);
        if control(progress) == TranscodeControl::Cancel {
            // SAFETY: packet is allocated and initialized.
            unsafe { ffi::av_packet_unref(input_packet.0) };
            return Ok(TranscodeOutcome::Cancelled(progress));
        }

        if packet_ref.stream_index == request.input_stream_index as i32 {
            send_packet(decoder_context.0, input_packet.0)?;
            // SAFETY: decoder has consumed or referenced the packet contents.
            unsafe { ffi::av_packet_unref(input_packet.0) };
            receive_and_encode_frames(
                decoder_context.0,
                &decoded_frame,
                &mut converter,
                encoder_context.0,
                &encoded_packet,
                output.context,
                output_stream,
                input_time_base,
                encoder_time_base,
                output_time_base,
                &mut next_pts,
                &mut progress,
            )?;
        } else {
            audio_states
                .iter_mut()
                .find(|audio| audio.input_stream_index == packet_ref.stream_index as u32)
                .expect("selected audio packets require audio state")
                .process_packet(input_packet.0, output.context, &mut progress)?;
            // SAFETY: decoder has consumed or referenced the packet contents.
            unsafe { ffi::av_packet_unref(input_packet.0) };
        }
    }

    send_packet(decoder_context.0, ptr::null())?;
    receive_and_encode_frames(
        decoder_context.0,
        &decoded_frame,
        &mut converter,
        encoder_context.0,
        &encoded_packet,
        output.context,
        output_stream,
        input_time_base,
        encoder_time_base,
        output_time_base,
        &mut next_pts,
        &mut progress,
    )?;
    send_frame(encoder_context.0, ptr::null())?;
    drain_encoder(
        encoder_context.0,
        &encoded_packet,
        output.context,
        output_stream,
        encoder_time_base,
        output_time_base,
        &mut progress,
    )?;
    for audio in &mut audio_states {
        audio.finish(output.context, &mut progress)?;
    }
    output.write_trailer()?;
    Ok(TranscodeOutcome::Completed(progress))
}

fn validate_request(request: &VideoTranscodeRequest) -> Result<()> {
    if request.decoder_name.trim().is_empty()
        || request.encoder_name.trim().is_empty()
        || request.output_pixel_format.trim().is_empty()
    {
        return Err(EngineError::invalid_argument(
            "video transcode decoder, encoder, and pixel format must be specified",
        ));
    }
    if request.target_bitrate_bps == Some(0) {
        return Err(EngineError::invalid_argument(
            "video target bitrate must be greater than zero",
        ));
    }
    validate_audio_stream_requests(&request.audio_streams)?;
    if request
        .audio_streams
        .iter()
        .any(|audio| audio.input_stream_index == request.input_stream_index)
    {
        return Err(EngineError::invalid_argument(
            "video and audio stream indexes must be distinct",
        ));
    }
    Ok(())
}

fn validate_audio_request(request: &AudioTranscodeRequest) -> Result<()> {
    if request.decoder_name.trim().is_empty() || request.encoder_name.trim().is_empty() {
        return Err(EngineError::invalid_argument(
            "audio transcode decoder and encoder must be specified",
        ));
    }
    if request.target_bitrate_bps == Some(0) {
        return Err(EngineError::invalid_argument(
            "audio target bitrate must be greater than zero",
        ));
    }
    if request.target_sample_rate_hz == Some(0) {
        return Err(EngineError::invalid_argument(
            "audio target sample rate must be greater than zero",
        ));
    }
    if request.target_channel_count == Some(0) {
        return Err(EngineError::invalid_argument(
            "audio target channel count must be greater than zero",
        ));
    }
    Ok(())
}

fn validate_audio_stream_requests(requests: &[AudioTranscodeRequest]) -> Result<()> {
    for request in requests {
        validate_audio_request(request)?;
    }
    if requests
        .windows(2)
        .any(|pair| pair[0].input_stream_index >= pair[1].input_stream_index)
    {
        return Err(EngineError::invalid_argument(
            "audio stream indexes must be unique and strictly increasing",
        ));
    }
    Ok(())
}

fn selected_video_stream(input: &InputContext, stream_index: u32) -> Result<&ffi::AVStream> {
    if stream_index >= input.as_ref().nb_streams {
        return Err(chain_not_ready("selected video stream does not exist"));
    }
    // SAFETY: stream_index is bounded by nb_streams.
    let stream = unsafe { &**input.as_ref().streams.add(stream_index as usize) };
    // SAFETY: codecpar is owned by the input stream.
    let parameters = unsafe { &*stream.codecpar };
    if parameters.codec_type != ffi::AVMEDIA_TYPE_VIDEO {
        return Err(chain_not_ready("selected stream is not video"));
    }
    Ok(stream)
}

fn selected_audio_stream(input: &InputContext, stream_index: u32) -> Result<&ffi::AVStream> {
    if stream_index >= input.as_ref().nb_streams {
        return Err(chain_not_ready("selected audio stream does not exist"));
    }
    // SAFETY: stream_index is bounded by nb_streams.
    let stream = unsafe { &**input.as_ref().streams.add(stream_index as usize) };
    // SAFETY: codecpar is owned by the input stream.
    let parameters = unsafe { &*stream.codecpar };
    if parameters.codec_type != ffi::AVMEDIA_TYPE_AUDIO {
        return Err(chain_not_ready("selected stream is not audio"));
    }
    Ok(stream)
}

fn validate_selected_input(
    input: &InputContext,
    video_stream_index: u32,
    audio_streams: &[AudioTranscodeRequest],
) -> Result<()> {
    selected_video_stream(input, video_stream_index)?;
    for audio in audio_streams {
        selected_audio_stream(input, audio.input_stream_index)?;
    }

    let mut video_count = 0;
    for index in 0..input.as_ref().nb_streams {
        // SAFETY: index is bounded by nb_streams.
        let stream = unsafe { &**input.as_ref().streams.add(index as usize) };
        // SAFETY: codecpar is owned by the input stream.
        match unsafe { (*stream.codecpar).codec_type } {
            ffi::AVMEDIA_TYPE_VIDEO => video_count += 1,
            ffi::AVMEDIA_TYPE_AUDIO => {}
            _ => {
                return Err(chain_not_ready(
                    "video transcoding currently supports video and audio streams only",
                ));
            }
        }
    }
    if video_count != 1 {
        return Err(chain_not_ready(
            "video transcoding requires exactly one video stream",
        ));
    }
    Ok(())
}

fn validate_audio_only_input(
    input: &InputContext,
    audio_streams: &[AudioTranscodeRequest],
) -> Result<()> {
    if audio_streams.is_empty() {
        return Err(EngineError::invalid_argument(
            "audio transcoding requires at least one selected audio stream",
        ));
    }
    for audio in audio_streams {
        selected_audio_stream(input, audio.input_stream_index)?;
    }
    for index in 0..input.as_ref().nb_streams {
        // SAFETY: index is bounded by nb_streams.
        let stream = unsafe { &**input.as_ref().streams.add(index as usize) };
        // SAFETY: codecpar is owned by the input stream.
        if unsafe { (*stream.codecpar).codec_type } != ffi::AVMEDIA_TYPE_AUDIO {
            return Err(chain_not_ready(
                "audio transcoding requires an audio-only input",
            ));
        }
    }
    Ok(())
}

fn c_string(value: &str, field: &str) -> Result<CString> {
    CString::new(value)
        .map_err(|_| EngineError::invalid_argument(format!("{field} contains a null byte")))
}

struct CodecContext(*mut ffi::AVCodecContext);

impl CodecContext {
    fn decoder(
        codec: *const ffi::AVCodec,
        parameters: *const ffi::AVCodecParameters,
    ) -> Result<Self> {
        // SAFETY: codec is a process-owned descriptor returned by libavcodec.
        let context = unsafe { ffi::avcodec_alloc_context3(codec) };
        if context.is_null() {
            return Err(native_allocation_error("video decoder context"));
        }
        let owned = Self(context);
        // SAFETY: both pointers are valid and context is exclusively owned.
        let copy_result = unsafe { ffi::avcodec_parameters_to_context(owned.0, parameters) };
        if copy_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to initialize video decoder parameters",
                copy_result,
            ));
        }
        // SAFETY: context and codec are compatible and options are omitted.
        let open_result = unsafe { ffi::avcodec_open2(owned.0, codec, ptr::null_mut()) };
        if open_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to open the selected video decoder",
                open_result,
            ));
        }
        if owned.as_ref().codec_type == ffi::AVMEDIA_TYPE_AUDIO
            && owned.as_ref().ch_layout.order == ffi::AV_CHANNEL_ORDER_UNSPEC
            && owned.as_ref().ch_layout.nb_channels > 0
        {
            let channel_count = owned.as_ref().ch_layout.nb_channels;
            // SAFETY: the decoder context owns this layout; default initializes a native layout.
            unsafe {
                ffi::av_channel_layout_uninit(&mut (*owned.0).ch_layout);
                ffi::av_channel_layout_default(&mut (*owned.0).ch_layout, channel_count);
            }
        }
        Ok(owned)
    }

    #[allow(clippy::too_many_arguments)]
    fn video_encoder(
        codec: *const ffi::AVCodec,
        decoder: &CodecContext,
        pixel_format: ffi::AVPixelFormat,
        frame_rate: ffi::AVRational,
        target_bitrate_bps: Option<u64>,
        profile: Option<&str>,
        global_header: bool,
    ) -> Result<Self> {
        // SAFETY: codec is a process-owned descriptor returned by libavcodec.
        let context = unsafe { ffi::avcodec_alloc_context3(codec) };
        if context.is_null() {
            return Err(native_allocation_error("video encoder context"));
        }
        let owned = Self(context);
        let decoder = decoder.as_ref();
        let frame_rate = valid_frame_rate(frame_rate);
        // SAFETY: context is exclusively owned and fields are initialized before open.
        unsafe {
            (*owned.0).width = decoder.width;
            (*owned.0).height = decoder.height;
            (*owned.0).sample_aspect_ratio = decoder.sample_aspect_ratio;
            (*owned.0).pix_fmt = pixel_format;
            (*owned.0).framerate = frame_rate;
            (*owned.0).time_base = ffi::av_inv_q(frame_rate);
            (*owned.0).bit_rate =
                target_bitrate_bps.unwrap_or(2_000_000).min(i64::MAX as u64) as i64;
            (*owned.0).gop_size = 48;
            (*owned.0).max_b_frames = 2;
            (*owned.0).color_range = decoder.color_range;
            (*owned.0).color_primaries = decoder.color_primaries;
            (*owned.0).color_trc = decoder.color_trc;
            (*owned.0).colorspace = decoder.colorspace;
            if global_header {
                (*owned.0).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        let mut options = ptr::null_mut();
        set_dictionary_option(&mut options, "preset", "medium")?;
        if let Some(profile) = profile {
            set_dictionary_option(&mut options, "profile", profile)?;
        }
        // SAFETY: context and codec are compatible; options is owned and freed below.
        let open_result = unsafe { ffi::avcodec_open2(owned.0, codec, &mut options) };
        // SAFETY: options is either null or owned by this call site.
        unsafe { ffi::av_dict_free(&mut options) };
        if open_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to open the selected video encoder",
                open_result,
            ));
        }
        Ok(owned)
    }

    fn audio_encoder(
        codec: *const ffi::AVCodec,
        decoder: &CodecContext,
        target_bitrate_bps: Option<u64>,
        target_sample_rate_hz: Option<u32>,
        target_channel_count: Option<u32>,
        global_header: bool,
    ) -> Result<Self> {
        let decoder = decoder.as_ref();
        if decoder.sample_rate <= 0 || decoder.ch_layout.nb_channels <= 0 {
            return Err(chain_not_ready(
                "decoded audio sample rate and channel layout must be available",
            ));
        }
        let sample_rate = target_sample_rate_hz
            .map(|value| value.min(i32::MAX as u32) as i32)
            .unwrap_or(decoder.sample_rate);
        let channel_count = target_channel_count
            .map(|value| value.min(i32::MAX as u32) as i32)
            .unwrap_or(decoder.ch_layout.nb_channels);
        if !codec_supports_sample_rate(codec, sample_rate)? {
            return Err(chain_not_ready(
                "selected audio encoder does not support the target sample rate",
            ));
        }
        // SAFETY: AVChannelLayout may be zero-initialized before av_channel_layout_default.
        let mut target_layout: ffi::AVChannelLayout = unsafe { std::mem::zeroed() };
        // SAFETY: target_layout is uninitialized and channel_count is positive.
        unsafe { ffi::av_channel_layout_default(&mut target_layout, channel_count) };
        if target_layout.nb_channels <= 0 || !codec_supports_channel_layout(codec, &target_layout)?
        {
            // SAFETY: target_layout was initialized above.
            unsafe { ffi::av_channel_layout_uninit(&mut target_layout) };
            return Err(chain_not_ready(
                "selected audio encoder does not support the target channel layout",
            ));
        }
        let sample_format = first_codec_sample_format(codec)?.ok_or_else(|| {
            chain_not_ready("selected audio encoder does not expose a sample format")
        })?;
        // SAFETY: codec is a process-owned descriptor returned by libavcodec.
        let context = unsafe { ffi::avcodec_alloc_context3(codec) };
        if context.is_null() {
            return Err(native_allocation_error("audio encoder context"));
        }
        let owned = Self(context);
        // SAFETY: context is exclusively owned and initialized before opening the encoder.
        unsafe {
            (*owned.0).sample_fmt = sample_format;
            (*owned.0).sample_rate = sample_rate;
            (*owned.0).time_base = ffi::AVRational {
                num: 1,
                den: sample_rate,
            };
            (*owned.0).bit_rate = target_bitrate_bps.unwrap_or(96_000).min(i64::MAX as u64) as i64;
            if global_header {
                (*owned.0).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        // SAFETY: both channel layouts are valid and the destination is uninitialized.
        let copy_result =
            unsafe { ffi::av_channel_layout_copy(&mut (*owned.0).ch_layout, &target_layout) };
        // SAFETY: target_layout is no longer needed after copying.
        unsafe { ffi::av_channel_layout_uninit(&mut target_layout) };
        if copy_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to configure the audio encoder channel layout",
                copy_result,
            ));
        }
        // SAFETY: context and codec are compatible and options are omitted.
        let open_result = unsafe { ffi::avcodec_open2(owned.0, codec, ptr::null_mut()) };
        if open_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to open the selected audio encoder",
                open_result,
            ));
        }
        Ok(owned)
    }

    fn as_ref(&self) -> &ffi::AVCodecContext {
        // SAFETY: CodecContext owns a non-null context until Drop.
        unsafe { &*self.0 }
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        // SAFETY: context is owned by this guard and freed exactly once.
        unsafe { ffi::avcodec_free_context(&mut self.0) };
    }
}

struct Frame(*mut ffi::AVFrame);

impl Frame {
    fn new() -> Result<Self> {
        // SAFETY: allocation has no pointer preconditions.
        let frame = unsafe { ffi::av_frame_alloc() };
        if frame.is_null() {
            return Err(native_allocation_error("video frame"));
        }
        Ok(Self(frame))
    }

    fn new_video(width: i32, height: i32, pixel_format: ffi::AVPixelFormat) -> Result<Self> {
        let frame = Self::new()?;
        // SAFETY: frame is exclusively owned and fields are initialized before allocation.
        unsafe {
            (*frame.0).width = width;
            (*frame.0).height = height;
            (*frame.0).format = pixel_format;
        }
        // SAFETY: frame dimensions and pixel format were initialized above.
        let result = unsafe { ffi::av_frame_get_buffer(frame.0, 32) };
        if result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaPixelFormatUnavailable,
                "failed to allocate converted video frame",
                result,
            ));
        }
        Ok(frame)
    }

    fn new_audio(context: &CodecContext, sample_count: i32) -> Result<Self> {
        if sample_count <= 0 {
            return Err(EngineError::invalid_argument(
                "audio frame sample count must be greater than zero",
            ));
        }
        let frame = Self::new()?;
        let encoder = context.as_ref();
        // SAFETY: frame is exclusively owned and initialized before buffer allocation.
        unsafe {
            (*frame.0).format = encoder.sample_fmt;
            (*frame.0).sample_rate = encoder.sample_rate;
            (*frame.0).nb_samples = sample_count;
        }
        // SAFETY: both layouts are valid and the frame destination is uninitialized.
        let copy_result =
            unsafe { ffi::av_channel_layout_copy(&mut (*frame.0).ch_layout, &encoder.ch_layout) };
        if copy_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to configure an audio frame channel layout",
                copy_result,
            ));
        }
        // SAFETY: audio frame parameters were initialized above.
        let buffer_result = unsafe { ffi::av_frame_get_buffer(frame.0, 0) };
        if buffer_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to allocate an audio frame buffer",
                buffer_result,
            ));
        }
        Ok(frame)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: frame is owned by this guard and freed exactly once.
        unsafe { ffi::av_frame_free(&mut self.0) };
    }
}

struct AudioResampler(*mut ffi::SwrContext);

fn audio_conversion_required(decoder: &CodecContext, encoder: &CodecContext) -> bool {
    let decoder = decoder.as_ref();
    let encoder = encoder.as_ref();
    decoder.sample_fmt != encoder.sample_fmt
        || decoder.sample_rate != encoder.sample_rate
        // SAFETY: both codec contexts own initialized channel layouts.
        || unsafe { ffi::av_channel_layout_compare(&decoder.ch_layout, &encoder.ch_layout) } != 0
}

impl AudioResampler {
    fn new(decoder: &CodecContext, encoder: &CodecContext) -> Result<Self> {
        let mut context = ptr::null_mut();
        let decoder = decoder.as_ref();
        let encoder = encoder.as_ref();
        // SAFETY: codec contexts expose initialized audio layouts and formats.
        let allocation_result = unsafe {
            ffi::swr_alloc_set_opts2(
                &mut context,
                &encoder.ch_layout,
                encoder.sample_fmt,
                encoder.sample_rate,
                &decoder.ch_layout,
                decoder.sample_fmt,
                decoder.sample_rate,
                0,
                ptr::null_mut(),
            )
        };
        if allocation_result < 0 || context.is_null() {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to configure the selected audio processor",
                allocation_result,
            ));
        }
        let owned = Self(context);
        // SAFETY: context was configured by swr_alloc_set_opts2.
        let initialize_result = unsafe { ffi::swr_init(owned.0) };
        if initialize_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaCapabilityIncompatible,
                "failed to initialize the selected audio processor",
                initialize_result,
            ));
        }
        Ok(owned)
    }

    fn convert_into_fifo(&mut self, input: *const ffi::AVFrame, fifo: &AudioFifo) -> Result<()> {
        // SAFETY: input was populated by avcodec_receive_frame.
        let input = unsafe { &*input };
        // SAFETY: resampler is initialized and input sample count is non-negative.
        let capacity = unsafe { ffi::swr_get_out_samples(self.0, input.nb_samples) };
        if capacity < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to size the converted audio frame",
                capacity,
            ));
        }
        if capacity == 0 {
            return Ok(());
        }
        let converted = Frame::new_audio(&fifo.encoder, capacity)?;
        // SAFETY: both input and output audio planes are valid for their sample counts.
        let sample_count = unsafe {
            ffi::swr_convert(
                self.0,
                (*converted.0).extended_data,
                capacity,
                input.extended_data as *const *const u8,
                input.nb_samples,
            )
        };
        if sample_count < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to convert decoded audio samples",
                sample_count,
            ));
        }
        fifo.write_frame(&converted, sample_count)
    }

    fn flush_into_fifo(&mut self, fifo: &AudioFifo) -> Result<()> {
        loop {
            // SAFETY: resampler is initialized; zero input requests its pending output bound.
            let capacity = unsafe { ffi::swr_get_out_samples(self.0, 0) };
            if capacity <= 0 {
                return if capacity == 0 {
                    Ok(())
                } else {
                    Err(native_execution_error(
                        EngineErrorCode::MediaInfoReadFailed,
                        "failed to size buffered audio samples",
                        capacity,
                    ))
                };
            }
            let converted = Frame::new_audio(&fifo.encoder, capacity)?;
            // SAFETY: a null input flushes pending samples into the allocated output frame.
            let sample_count = unsafe {
                ffi::swr_convert(
                    self.0,
                    (*converted.0).extended_data,
                    capacity,
                    ptr::null(),
                    0,
                )
            };
            if sample_count < 0 {
                return Err(native_execution_error(
                    EngineErrorCode::MediaInfoReadFailed,
                    "failed to flush converted audio samples",
                    sample_count,
                ));
            }
            if sample_count == 0 {
                return Ok(());
            }
            fifo.write_frame(&converted, sample_count)?;
        }
    }
}

impl Drop for AudioResampler {
    fn drop(&mut self) {
        // SAFETY: context is owned by this guard and freed exactly once.
        unsafe { ffi::swr_free(&mut self.0) };
    }
}

struct AudioFifo {
    fifo: *mut ffi::AVAudioFifo,
    encoder: CodecContext,
}

impl AudioFifo {
    fn new(encoder: CodecContext) -> Result<Self> {
        let context = encoder.as_ref();
        // SAFETY: encoder audio format and channel count were initialized before open.
        let fifo = unsafe {
            ffi::av_audio_fifo_alloc(context.sample_fmt, context.ch_layout.nb_channels, 1)
        };
        if fifo.is_null() {
            return Err(native_allocation_error("audio sample FIFO"));
        }
        Ok(Self { fifo, encoder })
    }

    fn size(&self) -> i32 {
        // SAFETY: fifo is owned and remains valid.
        unsafe { ffi::av_audio_fifo_size(self.fifo) }
    }

    fn write_frame(&self, frame: &Frame, sample_count: i32) -> Result<()> {
        if sample_count == 0 {
            return Ok(());
        }
        // SAFETY: converted frame contains sample_count initialized audio samples.
        let written = unsafe {
            ffi::av_audio_fifo_write(
                self.fifo,
                (*frame.0).extended_data as *const *mut c_void,
                sample_count,
            )
        };
        if written != sample_count {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to buffer converted audio samples",
                written,
            ));
        }
        Ok(())
    }

    fn read_frame(&self, frame: &Frame, sample_count: i32) -> Result<()> {
        // SAFETY: frame owns enough writable audio planes for sample_count samples.
        let read = unsafe {
            ffi::av_audio_fifo_read(
                self.fifo,
                (*frame.0).extended_data as *const *mut c_void,
                sample_count,
            )
        };
        if read != sample_count {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to read converted audio samples",
                read,
            ));
        }
        Ok(())
    }
}

impl Drop for AudioFifo {
    fn drop(&mut self) {
        // SAFETY: fifo is owned by this guard and freed exactly once.
        unsafe { ffi::av_audio_fifo_free(self.fifo) };
    }
}

struct AudioTranscodeState {
    input_stream_index: u32,
    input_time_base: ffi::AVRational,
    decoder: CodecContext,
    fifo: AudioFifo,
    resampler: Option<AudioResampler>,
    output_stream: *mut ffi::AVStream,
    decoded_frame: Frame,
    encoded_packet: Packet,
    next_pts: Option<i64>,
}

impl AudioTranscodeState {
    fn new(
        request: &AudioTranscodeRequest,
        input_stream: &ffi::AVStream,
        output: &mut OutputContext,
        global_header: bool,
    ) -> Result<Self> {
        let decoder_name = c_string(&request.decoder_name, "audio decoder name")?;
        // SAFETY: decoder_name is a valid null-terminated codec name.
        let decoder = unsafe { ffi::avcodec_find_decoder_by_name(decoder_name.as_ptr()) };
        if decoder.is_null() {
            return Err(chain_not_ready(format!(
                "selected audio decoder {} is unavailable",
                request.decoder_name
            )));
        }
        let decoder_context = CodecContext::decoder(decoder, input_stream.codecpar)?;
        // SAFETY: decoder context is exclusively owned and stream time base is valid.
        unsafe { (*decoder_context.0).pkt_timebase = input_stream.time_base };

        let encoder_name = c_string(&request.encoder_name, "audio encoder name")?;
        // SAFETY: encoder_name is a valid null-terminated codec name.
        let encoder = unsafe { ffi::avcodec_find_encoder_by_name(encoder_name.as_ptr()) };
        if encoder.is_null() {
            return Err(chain_not_ready(format!(
                "selected audio encoder {} is unavailable",
                request.encoder_name
            )));
        }
        let encoder_context = CodecContext::audio_encoder(
            encoder,
            &decoder_context,
            request.target_bitrate_bps,
            request.target_sample_rate_hz,
            request.target_channel_count,
            global_header,
        )?;
        let output_stream = create_output_stream(output, &encoder_context, "audio")?;
        let resampler = audio_conversion_required(&decoder_context, &encoder_context)
            .then(|| AudioResampler::new(&decoder_context, &encoder_context))
            .transpose()?;
        let fifo = AudioFifo::new(encoder_context)?;
        Ok(Self {
            input_stream_index: request.input_stream_index,
            input_time_base: input_stream.time_base,
            decoder: decoder_context,
            fifo,
            resampler,
            output_stream,
            decoded_frame: Frame::new()?,
            encoded_packet: Packet::new_for_execution()?,
            next_pts: None,
        })
    }

    fn process_packet(
        &mut self,
        packet: *const ffi::AVPacket,
        output: *mut ffi::AVFormatContext,
        progress: &mut TranscodeProgress,
    ) -> Result<()> {
        send_packet(self.decoder.0, packet)?;
        self.receive_frames(output, progress)
    }

    fn receive_frames(
        &mut self,
        output: *mut ffi::AVFormatContext,
        progress: &mut TranscodeProgress,
    ) -> Result<()> {
        loop {
            // SAFETY: decoder and frame are valid and exclusively used here.
            let result =
                unsafe { ffi::avcodec_receive_frame(self.decoder.0, self.decoded_frame.0) };
            if result == ffi::AVERROR(ffi::EAGAIN) || result == ffi::AVERROR_EOF {
                return Ok(());
            }
            if result < 0 {
                return Err(native_execution_error(
                    EngineErrorCode::MediaInfoReadFailed,
                    "failed to receive a decoded audio frame",
                    result,
                ));
            }
            if self.next_pts.is_none() {
                // SAFETY: decoded frame was populated by avcodec_receive_frame.
                let frame = unsafe { &*self.decoded_frame.0 };
                let timestamp = if frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE {
                    frame.best_effort_timestamp
                } else {
                    frame.pts
                };
                let encoder_time_base = self.fifo.encoder.as_ref().time_base;
                self.next_pts = Some(if timestamp == ffi::AV_NOPTS_VALUE {
                    0
                } else {
                    // SAFETY: input and encoder time bases are valid.
                    unsafe { ffi::av_rescale_q(timestamp, self.input_time_base, encoder_time_base) }
                        .max(0)
                });
            }
            if let Some(resampler) = self.resampler.as_mut() {
                resampler.convert_into_fifo(self.decoded_frame.0, &self.fifo)?;
            } else {
                // SAFETY: the decoder populated the frame and no format conversion is required.
                let sample_count = unsafe { (*self.decoded_frame.0).nb_samples };
                self.fifo.write_frame(&self.decoded_frame, sample_count)?;
            }
            self.drain_fifo(output, progress, false)?;
            // SAFETY: decoded frame may release references for reuse.
            unsafe { ffi::av_frame_unref(self.decoded_frame.0) };
        }
    }

    fn drain_fifo(
        &mut self,
        output: *mut ffi::AVFormatContext,
        progress: &mut TranscodeProgress,
        finishing: bool,
    ) -> Result<()> {
        let frame_size = self.fifo.encoder.as_ref().frame_size.max(1);
        loop {
            let available = self.fifo.size();
            if available < frame_size && !finishing || available == 0 {
                return Ok(());
            }
            let sample_count = available.min(frame_size);
            let frame = Frame::new_audio(&self.fifo.encoder, sample_count)?;
            self.fifo.read_frame(&frame, sample_count)?;
            // SAFETY: frame is exclusively owned and timestamps use encoder time base.
            let pts = self.next_pts.unwrap_or(0);
            unsafe { (*frame.0).pts = pts };
            self.next_pts = Some(pts.saturating_add(i64::from(sample_count)));
            send_frame(self.fifo.encoder.0, frame.0)?;
            let encoder_time_base = self.fifo.encoder.as_ref().time_base;
            // SAFETY: output stream belongs to the live output format context.
            let output_time_base = unsafe { (*self.output_stream).time_base };
            drain_encoder(
                self.fifo.encoder.0,
                &self.encoded_packet,
                output,
                self.output_stream,
                encoder_time_base,
                output_time_base,
                progress,
            )?;
        }
    }

    fn finish(
        &mut self,
        output: *mut ffi::AVFormatContext,
        progress: &mut TranscodeProgress,
    ) -> Result<()> {
        send_packet(self.decoder.0, ptr::null())?;
        self.receive_frames(output, progress)?;
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.flush_into_fifo(&self.fifo)?;
        }
        self.drain_fifo(output, progress, true)?;
        send_frame(self.fifo.encoder.0, ptr::null())?;
        let encoder_time_base = self.fifo.encoder.as_ref().time_base;
        // SAFETY: output stream belongs to the live output format context.
        let output_time_base = unsafe { (*self.output_stream).time_base };
        drain_encoder(
            self.fifo.encoder.0,
            &self.encoded_packet,
            output,
            self.output_stream,
            encoder_time_base,
            output_time_base,
            progress,
        )
    }
}

#[derive(Default)]
struct FrameConverter {
    context: *mut ffi::SwsContext,
    source_width: i32,
    source_height: i32,
    source_format: ffi::AVPixelFormat,
    output: Option<Frame>,
}

impl FrameConverter {
    fn convert(
        &mut self,
        source: *const ffi::AVFrame,
        target_format: ffi::AVPixelFormat,
    ) -> Result<*const ffi::AVFrame> {
        // SAFETY: source was populated by avcodec_receive_frame.
        let source_ref = unsafe { &*source };
        if source_ref.format == target_format {
            return Ok(source);
        }
        let source_format = source_ref.format as ffi::AVPixelFormat;
        if self.context.is_null()
            || self.source_width != source_ref.width
            || self.source_height != source_ref.height
            || self.source_format != source_format
        {
            self.reset();
            // SAFETY: dimensions and pixel formats come from decoder/encoder contexts.
            self.context = unsafe {
                ffi::sws_getContext(
                    source_ref.width,
                    source_ref.height,
                    source_format,
                    source_ref.width,
                    source_ref.height,
                    target_format,
                    ffi::SWS_BICUBIC as i32,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            if self.context.is_null() {
                return Err(chain_not_ready(
                    "failed to initialize the selected video processor",
                ));
            }
            self.output = Some(Frame::new_video(
                source_ref.width,
                source_ref.height,
                target_format,
            )?);
            self.source_width = source_ref.width;
            self.source_height = source_ref.height;
            self.source_format = source_format;
        }
        let output = self
            .output
            .as_ref()
            .expect("converter output is allocated with its context");
        // SAFETY: output frame owns writable buffers.
        let writable = unsafe { ffi::av_frame_make_writable(output.0) };
        if writable < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaPixelFormatUnavailable,
                "converted video frame is not writable",
                writable,
            ));
        }
        // SAFETY: source data is decoder-owned and output buffers match the target format.
        let scaled = unsafe {
            ffi::sws_scale(
                self.context,
                source_ref.data.as_ptr() as *const *const u8,
                source_ref.linesize.as_ptr(),
                0,
                source_ref.height,
                (*output.0).data.as_ptr(),
                (*output.0).linesize.as_ptr(),
            )
        };
        if scaled != source_ref.height {
            return Err(chain_not_ready(
                "selected video processor returned an incomplete frame",
            ));
        }
        // SAFETY: both frames are valid and output is exclusively owned.
        unsafe {
            (*output.0).pts = source_ref.pts;
            (*output.0).sample_aspect_ratio = source_ref.sample_aspect_ratio;
            (*output.0).color_range = source_ref.color_range;
            (*output.0).color_primaries = source_ref.color_primaries;
            (*output.0).color_trc = source_ref.color_trc;
            (*output.0).colorspace = source_ref.colorspace;
        }
        Ok(output.0)
    }

    fn reset(&mut self) {
        if !self.context.is_null() {
            // SAFETY: context is owned by this converter and freed exactly once.
            unsafe { ffi::sws_freeContext(self.context) };
            self.context = ptr::null_mut();
        }
        self.output = None;
    }
}

impl Drop for FrameConverter {
    fn drop(&mut self) {
        self.reset();
    }
}

#[allow(clippy::too_many_arguments)]
fn receive_and_encode_frames(
    decoder: *mut ffi::AVCodecContext,
    decoded_frame: &Frame,
    converter: &mut FrameConverter,
    encoder: *mut ffi::AVCodecContext,
    encoded_packet: &Packet,
    output: *mut ffi::AVFormatContext,
    output_stream: *mut ffi::AVStream,
    input_time_base: ffi::AVRational,
    encoder_time_base: ffi::AVRational,
    output_time_base: ffi::AVRational,
    next_pts: &mut i64,
    progress: &mut TranscodeProgress,
) -> Result<()> {
    loop {
        // SAFETY: decoder and frame are valid and exclusively used here.
        let result = unsafe { ffi::avcodec_receive_frame(decoder, decoded_frame.0) };
        if result == ffi::AVERROR(ffi::EAGAIN) || result == ffi::AVERROR_EOF {
            return Ok(());
        }
        if result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to receive a decoded video frame",
                result,
            ));
        }
        // SAFETY: avcodec_receive_frame populated the frame.
        let frame_ref = unsafe { &mut *decoded_frame.0 };
        let source_timestamp = if frame_ref.best_effort_timestamp != ffi::AV_NOPTS_VALUE {
            frame_ref.best_effort_timestamp
        } else {
            frame_ref.pts
        };
        let pts = if source_timestamp == ffi::AV_NOPTS_VALUE {
            *next_pts
        } else {
            // SAFETY: stream and encoder time bases are valid.
            unsafe { ffi::av_rescale_q(source_timestamp, input_time_base, encoder_time_base) }
                .max(*next_pts)
        };
        frame_ref.pts = pts;
        *next_pts = pts.saturating_add(1);
        let converted = converter.convert(decoded_frame.0, unsafe { (*encoder).pix_fmt })?;
        send_frame(encoder, converted)?;
        progress.decoded_frames = progress.decoded_frames.saturating_add(1);
        drain_encoder(
            encoder,
            encoded_packet,
            output,
            output_stream,
            encoder_time_base,
            output_time_base,
            progress,
        )?;
        // SAFETY: decoded frame is allocated and may release references for reuse.
        unsafe { ffi::av_frame_unref(decoded_frame.0) };
    }
}

fn drain_encoder(
    encoder: *mut ffi::AVCodecContext,
    encoded_packet: &Packet,
    output: *mut ffi::AVFormatContext,
    output_stream: *mut ffi::AVStream,
    encoder_time_base: ffi::AVRational,
    output_time_base: ffi::AVRational,
    progress: &mut TranscodeProgress,
) -> Result<()> {
    loop {
        // SAFETY: encoder and packet are valid and exclusively used here.
        let result = unsafe { ffi::avcodec_receive_packet(encoder, encoded_packet.0) };
        if result == ffi::AVERROR(ffi::EAGAIN) || result == ffi::AVERROR_EOF {
            return Ok(());
        }
        if result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::MediaInfoReadFailed,
                "failed to receive an encoded video packet",
                result,
            ));
        }
        // SAFETY: packet and output stream are valid.
        unsafe {
            ffi::av_packet_rescale_ts(encoded_packet.0, encoder_time_base, output_time_base);
            (*encoded_packet.0).stream_index = (*output_stream).index;
        }
        // SAFETY: output header was written and packet timestamps use output time base.
        let write_result = unsafe { ffi::av_interleaved_write_frame(output, encoded_packet.0) };
        if write_result < 0 {
            return Err(native_execution_error(
                EngineErrorCode::OutputContainerNotWritable,
                "failed to write encoded video packet",
                write_result,
            ));
        }
        progress.encoded_packets = progress.encoded_packets.saturating_add(1);
        // SAFETY: packet is allocated and may release references for reuse.
        unsafe { ffi::av_packet_unref(encoded_packet.0) };
    }
}

fn send_packet(decoder: *mut ffi::AVCodecContext, packet: *const ffi::AVPacket) -> Result<()> {
    // SAFETY: decoder is open and packet is either valid or null for flush.
    let result = unsafe { ffi::avcodec_send_packet(decoder, packet) };
    if result < 0 && result != ffi::AVERROR_EOF {
        return Err(native_execution_error(
            EngineErrorCode::MediaInfoReadFailed,
            "failed to send a packet to the video decoder",
            result,
        ));
    }
    Ok(())
}

fn send_frame(encoder: *mut ffi::AVCodecContext, frame: *const ffi::AVFrame) -> Result<()> {
    // SAFETY: encoder is open and frame is either valid or null for flush.
    let result = unsafe { ffi::avcodec_send_frame(encoder, frame) };
    if result < 0 && result != ffi::AVERROR_EOF {
        return Err(native_execution_error(
            EngineErrorCode::MediaInfoReadFailed,
            "failed to send a frame to the video encoder",
            result,
        ));
    }
    Ok(())
}

fn create_output_stream(
    output: &mut OutputContext,
    encoder: &CodecContext,
    stream_kind: &str,
) -> Result<*mut ffi::AVStream> {
    // SAFETY: output owns a valid format context.
    let stream = unsafe { ffi::avformat_new_stream(output.context, ptr::null()) };
    if stream.is_null() {
        return Err(EngineError::with_code(
            ErrorKind::Runtime,
            EngineErrorCode::OutputContainerNotWritable,
            format!("failed to create encoded {stream_kind} output stream"),
        ));
    }
    // SAFETY: stream codec parameters and encoder context are valid.
    let result = unsafe { ffi::avcodec_parameters_from_context((*stream).codecpar, encoder.0) };
    if result < 0 {
        return Err(native_execution_error(
            EngineErrorCode::OutputContainerNotWritable,
            "failed to export encoded video parameters",
            result,
        ));
    }
    // SAFETY: stream is owned by output and exclusively initialized here.
    unsafe {
        (*(*stream).codecpar).codec_tag = 0;
        (*stream).time_base = encoder.as_ref().time_base;
    }
    Ok(stream)
}

fn output_requires_global_header(output: &OutputContext) -> bool {
    // SAFETY: output format is initialized with the context.
    unsafe { (*output.as_ref().oformat).flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 }
}

fn input_frame_rate(input: &InputContext, stream: &ffi::AVStream) -> ffi::AVRational {
    // SAFETY: input and stream belong to the same live format context.
    let guessed =
        unsafe { ffi::av_guess_frame_rate(input.0, stream as *const _ as *mut _, ptr::null_mut()) };
    if guessed.num > 0 && guessed.den > 0 {
        guessed
    } else if stream.avg_frame_rate.num > 0 && stream.avg_frame_rate.den > 0 {
        stream.avg_frame_rate
    } else if stream.r_frame_rate.num > 0 && stream.r_frame_rate.den > 0 {
        stream.r_frame_rate
    } else {
        ffi::AVRational { num: 30, den: 1 }
    }
}

fn valid_frame_rate(value: ffi::AVRational) -> ffi::AVRational {
    if value.num > 0 && value.den > 0 {
        value
    } else {
        ffi::AVRational { num: 30, den: 1 }
    }
}

fn codec_supports_pixel_format(
    codec: *const ffi::AVCodec,
    pixel_format: ffi::AVPixelFormat,
) -> Result<bool> {
    let Some((configs, config_count)) = codec_supported_config(
        codec,
        ffi::AV_CODEC_CONFIG_PIX_FORMAT,
        "failed to query encoder pixel-format capabilities",
    )?
    else {
        // Preserve the previous conservative policy for an unrestricted/null list.
        return Ok(false);
    };
    let formats = configs.cast::<ffi::AVPixelFormat>();
    for index in 0..config_count {
        // SAFETY: libavcodec returned config_count elements in the capability array.
        if unsafe { *formats.add(index) } == pixel_format {
            return Ok(true);
        }
    }
    Ok(false)
}

fn codec_supported_config(
    codec: *const ffi::AVCodec,
    config: ffi::AVCodecConfig,
    description: &str,
) -> Result<Option<(*const c_void, usize)>> {
    let mut configs = ptr::null();
    let mut config_count = 0;
    // SAFETY: codec is a valid process-owned descriptor; the output pointers are local and the
    // returned configuration memory is owned by libavcodec for the lifetime of the codec.
    let result = unsafe {
        ffi::avcodec_get_supported_config(
            ptr::null(),
            codec,
            config,
            0,
            &mut configs,
            &mut config_count,
        )
    };
    if result < 0 {
        return Err(native_execution_error(
            EngineErrorCode::MediaCapabilityIncompatible,
            description,
            result,
        ));
    }
    if configs.is_null() {
        return Ok(None);
    }
    if config_count < 0 {
        return Err(EngineError::with_code(
            ErrorKind::NativeLibrary,
            EngineErrorCode::MediaCapabilityIncompatible,
            "FFmpeg returned a negative codec capability count",
        ));
    }
    Ok(Some((configs, config_count as usize)))
}

fn first_codec_sample_format(codec: *const ffi::AVCodec) -> Result<Option<ffi::AVSampleFormat>> {
    let Some((configs, config_count)) = codec_supported_config(
        codec,
        ffi::AV_CODEC_CONFIG_SAMPLE_FORMAT,
        "failed to query encoder sample-format capabilities",
    )?
    else {
        return Ok(None);
    };
    if config_count == 0 {
        return Ok(None);
    }
    let formats = configs.cast::<ffi::AVSampleFormat>();
    // SAFETY: libavcodec returned at least one sample format.
    let format = unsafe { *formats };
    Ok((format != ffi::AV_SAMPLE_FMT_NONE).then_some(format))
}

fn codec_supports_sample_rate(codec: *const ffi::AVCodec, sample_rate: i32) -> Result<bool> {
    let Some((configs, config_count)) = codec_supported_config(
        codec,
        ffi::AV_CODEC_CONFIG_SAMPLE_RATE,
        "failed to query encoder sample-rate capabilities",
    )?
    else {
        return Ok(true);
    };
    let rates = configs.cast::<i32>();
    for index in 0..config_count {
        // SAFETY: libavcodec returned config_count elements in the capability array.
        if unsafe { *rates.add(index) } == sample_rate {
            return Ok(true);
        }
    }
    Ok(false)
}

fn codec_supports_channel_layout(
    codec: *const ffi::AVCodec,
    layout: &ffi::AVChannelLayout,
) -> Result<bool> {
    let Some((configs, config_count)) = codec_supported_config(
        codec,
        ffi::AV_CODEC_CONFIG_CHANNEL_LAYOUT,
        "failed to query encoder channel-layout capabilities",
    )?
    else {
        return Ok(true);
    };
    let layouts = configs.cast::<ffi::AVChannelLayout>();
    for index in 0..config_count {
        // SAFETY: libavcodec returned config_count initialized channel layouts.
        let candidate = unsafe { &*layouts.add(index) };
        // SAFETY: both layouts are initialized and valid.
        if unsafe { ffi::av_channel_layout_compare(candidate, layout) } == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn set_dictionary_option(
    dictionary: &mut *mut ffi::AVDictionary,
    key: &str,
    value: &str,
) -> Result<()> {
    let key = c_string(key, "encoder option key")?;
    let value = c_string(value, "encoder option value")?;
    // SAFETY: dictionary is an owned out pointer and strings live for the call.
    let result = unsafe { ffi::av_dict_set(dictionary, key.as_ptr(), value.as_ptr(), 0) };
    if result < 0 {
        return Err(native_execution_error(
            EngineErrorCode::MediaCapabilityIncompatible,
            "failed to configure the selected video encoder",
            result,
        ));
    }
    Ok(())
}

fn update_input_progress(
    progress: &mut TranscodeProgress,
    packet: &ffi::AVPacket,
    time_base: ffi::AVRational,
) {
    let timestamp = if packet.pts != ffi::AV_NOPTS_VALUE {
        packet.pts
    } else {
        packet.dts
    };
    if timestamp != ffi::AV_NOPTS_VALUE {
        // SAFETY: packet and microsecond time bases are valid.
        let media_time_us =
            unsafe { ffi::av_rescale_q(timestamp, time_base, MICROSECONDS_TIME_BASE) };
        progress.media_time_us = progress.media_time_us.max(media_time_us.max(0) as u64);
    }
    progress.processed_bytes = progress
        .processed_bytes
        .saturating_add(packet.size.max(0) as u64);
}

fn native_allocation_error(resource: &str) -> EngineError {
    EngineError::with_code(
        ErrorKind::Runtime,
        EngineErrorCode::NativeLibraryUnavailable,
        format!("failed to allocate {resource}"),
    )
}

fn chain_not_ready(message: impl Into<String>) -> EngineError {
    EngineError::with_code(
        ErrorKind::Pipeline,
        EngineErrorCode::EngineExecutionChainNotReady,
        message,
    )
}

fn native_execution_error(error_code: EngineErrorCode, message: &str, code: i32) -> EngineError {
    let mut buffer = [0_i8; 256];
    // SAFETY: buffer is writable for its full length.
    let description = unsafe {
        if ffi::av_strerror(code, buffer.as_mut_ptr(), buffer.len()) == 0 {
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        } else {
            format!("FFmpeg error {code}")
        }
    };
    EngineError::with_code(
        ErrorKind::Runtime,
        error_code,
        format!("{message}: {description}"),
    )
}
