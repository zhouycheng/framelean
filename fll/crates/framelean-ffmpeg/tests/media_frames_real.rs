use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use framelean_analysis::{
    MediaAnalyzeRequest, MediaAnalyzer, MediaDescriptor, MediaSource, MediaStreamDescriptor,
};
use framelean_ffmpeg::{
    AudioFileTranscodeRequest, AudioTranscodeRequest, FfmpegAdapter, PreviewFramesRequest,
    TranscodeControl, TranscodeOutcome, VideoThumbnailRequest, VideoTranscodeRequest,
};

#[test]
fn real_libav_video_generates_preview_frames_and_a_visible_thumbnail() {
    let fixture = TestDirectory::new("media-frames");
    let input_path = fixture.path.join("three-frames.avi");
    fs::write(&input_path, uncompressed_avi_fixture()).unwrap();

    let adapter = FfmpegAdapter::new().unwrap();
    let preview_directory = fixture.path.join("previews");
    let previews = adapter
        .generate_preview_frames(&PreviewFramesRequest {
            input_path: input_path.clone(),
            output_directory: preview_directory.clone(),
            timestamps_us: vec![0, 1_000_000],
            max_width: Some(1),
        })
        .unwrap();

    assert_eq!(previews.output_directory, preview_directory);
    assert_eq!(previews.frames.len(), 2);
    assert_eq!(previews.frames[0].requested_timestamp_us, 0);
    assert_eq!(previews.frames[1].requested_timestamp_us, 1_000_000);
    for frame in &previews.frames {
        assert_eq!((frame.width, frame.height), (1, 1));
        assert_valid_bmp(&frame.output_path, 1, 1);
    }
    assert!(fs::read_dir(&preview_directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")
    }));

    let thumbnail_path = fixture.path.join("thumbnail.bmp");
    let thumbnail = adapter
        .generate_video_thumbnail(&VideoThumbnailRequest {
            input_path,
            output_path: thumbnail_path.clone(),
            duration_us: Some(1_500_000),
            max_width: 1,
        })
        .unwrap();

    assert_eq!(thumbnail.output_path, thumbnail_path);
    assert_eq!((thumbnail.width, thumbnail.height), (1, 1));
    assert!(thumbnail.requested_timestamp_us >= 100_000);
    let thumbnail_bytes = assert_valid_bmp(&thumbnail.output_path, 1, 1);
    assert!(
        thumbnail_bytes[54..].iter().any(|value| *value != 0),
        "thumbnail policy must skip the black frames and publish a visible frame"
    );
    assert!(
        !thumbnail
            .output_path
            .with_extension(format!("tmp-{}", std::process::id()))
            .exists()
    );
}

#[test]
fn real_libav_decodes_converts_encodes_and_muxes_video_without_cli() {
    let fixture = TestDirectory::new("video-transcode");
    let input_path = fixture.path.join("input.avi");
    let output_path = fixture.path.join("output.mp4");
    fs::write(&input_path, uncompressed_avi_fixture()).unwrap();

    let adapter = FfmpegAdapter::new().unwrap();
    let mut progress_events = Vec::new();
    let outcome = adapter
        .transcode_video(
            &VideoTranscodeRequest {
                input_path,
                output_path: output_path.clone(),
                input_stream_index: 0,
                decoder_name: "rawvideo".to_owned(),
                encoder_name: "libx264".to_owned(),
                output_pixel_format: "yuv420p".to_owned(),
                output_profile: Some("main".to_owned()),
                target_bitrate_bps: Some(180_000),
                audio_streams: Vec::new(),
            },
            |progress| {
                progress_events.push(progress);
                TranscodeControl::Continue
            },
        )
        .unwrap();

    let TranscodeOutcome::Completed(progress) = outcome else {
        panic!("video transcode unexpectedly cancelled");
    };
    assert!(progress.processed_bytes > 0);
    assert!(progress.decoded_frames >= 3);
    assert!(progress.encoded_packets > 0);
    assert!(!progress_events.is_empty());
    assert!(fs::metadata(&output_path).unwrap().len() > 64);

    let previews = adapter
        .generate_preview_frames(&PreviewFramesRequest {
            input_path: output_path,
            output_directory: fixture.path.join("transcoded-previews"),
            timestamps_us: vec![0],
            max_width: Some(2),
        })
        .unwrap();
    assert_eq!(previews.frames.len(), 1);
    assert_valid_bmp(&previews.frames[0].output_path, 2, 2);
}

#[test]
fn real_libav_transcodes_video_and_audio_into_one_mp4_without_cli() {
    let fixture = TestDirectory::new("audiovisual-transcode");
    let input_path = fixture.path.join("input.avi");
    let output_path = fixture.path.join("output.mp4");
    fs::write(&input_path, uncompressed_audiovisual_avi_fixture()).unwrap();

    let adapter = FfmpegAdapter::new().unwrap();
    let outcome = adapter
        .transcode_video(
            &VideoTranscodeRequest {
                input_path,
                output_path: output_path.clone(),
                input_stream_index: 0,
                decoder_name: "rawvideo".to_owned(),
                encoder_name: "libx264".to_owned(),
                output_pixel_format: "yuv420p".to_owned(),
                output_profile: Some("main".to_owned()),
                target_bitrate_bps: Some(180_000),
                audio_streams: vec![AudioTranscodeRequest {
                    input_stream_index: 1,
                    decoder_name: "pcm_s16le".to_owned(),
                    encoder_name: "aac".to_owned(),
                    target_bitrate_bps: Some(64_000),
                    target_sample_rate_hz: None,
                    target_channel_count: None,
                }],
            },
            |_| TranscodeControl::Continue,
        )
        .unwrap();

    let TranscodeOutcome::Completed(progress) = outcome else {
        panic!("audio/video transcode unexpectedly cancelled");
    };
    assert!(progress.decoded_frames >= 3);
    assert!(progress.encoded_packets >= 2);
    assert!(fs::metadata(&output_path).unwrap().len() > 128);

    let analyzed = adapter
        .analyze(&MediaAnalyzeRequest {
            source: MediaSource::local_file(&output_path).unwrap(),
            request_id: Some("audiovisual-output".to_owned()),
            expected_source: None,
        })
        .unwrap();
    let (media, _) = analyzed.into_parts();
    let MediaDescriptor::Video { streams } = media.descriptor else {
        panic!("transcoded output must remain a video");
    };
    assert_eq!(streams.len(), 2);
    assert!(streams.iter().any(|stream| {
        matches!(stream, MediaStreamDescriptor::Video(video) if video.codec == "h264")
    }));
    assert!(streams.iter().any(|stream| {
        matches!(stream, MediaStreamDescriptor::Audio(audio) if audio.codec == "aac")
    }));
}

#[test]
fn real_libav_transcodes_two_audio_streams_with_independent_parameters_without_cli() {
    let fixture = TestDirectory::new("multi-audio-video-transcode");
    let input_path = fixture.path.join("input.avi");
    let output_path = fixture.path.join("output.mp4");
    fs::write(&input_path, uncompressed_audiovisual_avi_fixture()).unwrap();

    let adapter = FfmpegAdapter::new().unwrap();
    adapter
        .transcode_video(
            &VideoTranscodeRequest {
                input_path,
                output_path: output_path.clone(),
                input_stream_index: 0,
                decoder_name: "rawvideo".to_owned(),
                encoder_name: "libx264".to_owned(),
                output_pixel_format: "yuv420p".to_owned(),
                output_profile: Some("main".to_owned()),
                target_bitrate_bps: Some(180_000),
                audio_streams: vec![
                    AudioTranscodeRequest {
                        input_stream_index: 1,
                        decoder_name: "pcm_s16le".to_owned(),
                        encoder_name: "aac".to_owned(),
                        target_bitrate_bps: Some(64_000),
                        target_sample_rate_hz: Some(32_000),
                        target_channel_count: Some(1),
                    },
                    AudioTranscodeRequest {
                        input_stream_index: 2,
                        decoder_name: "pcm_s16le".to_owned(),
                        encoder_name: "aac".to_owned(),
                        target_bitrate_bps: Some(96_000),
                        target_sample_rate_hz: Some(48_000),
                        target_channel_count: Some(2),
                    },
                ],
            },
            |_| TranscodeControl::Continue,
        )
        .unwrap();

    let analyzed = adapter
        .analyze(&MediaAnalyzeRequest {
            source: MediaSource::local_file(&output_path).unwrap(),
            request_id: Some("multi-audio-output".to_owned()),
            expected_source: None,
        })
        .unwrap();
    let (media, _) = analyzed.into_parts();
    let MediaDescriptor::Video { streams } = media.descriptor else {
        panic!("transcoded output must remain a video");
    };
    let audio_streams: Vec<_> = streams
        .iter()
        .filter_map(|stream| match stream {
            MediaStreamDescriptor::Audio(audio) => Some(audio.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(audio_streams.len(), 2);
    assert_eq!(audio_streams[0].codec, "aac");
    assert_eq!(audio_streams[0].sample_rate_hz.value, Some(32_000));
    assert_eq!(audio_streams[0].channel_count.value, Some(1));
    assert_eq!(audio_streams[1].codec, "aac");
    assert_eq!(audio_streams[1].sample_rate_hz.value, Some(48_000));
    assert_eq!(audio_streams[1].channel_count.value, Some(2));
}

#[test]
fn real_libav_transcodes_pcm_audio_into_aac_m4a_without_cli() {
    let fixture = TestDirectory::new("audio-transcode");
    let input_path = fixture.path.join("input.wav");
    let output_path = fixture.path.join("output.m4a");
    fs::write(&input_path, pcm_wav_fixture()).unwrap();

    let adapter = FfmpegAdapter::new().unwrap();
    let outcome = adapter
        .transcode_audio(
            &AudioFileTranscodeRequest {
                input_path,
                output_path: output_path.clone(),
                audio_streams: vec![AudioTranscodeRequest {
                    input_stream_index: 0,
                    decoder_name: "pcm_s16le".to_owned(),
                    encoder_name: "aac".to_owned(),
                    target_bitrate_bps: Some(192_000),
                    target_sample_rate_hz: Some(32_000),
                    target_channel_count: Some(2),
                }],
            },
            |_| TranscodeControl::Continue,
        )
        .unwrap();

    let TranscodeOutcome::Completed(progress) = outcome else {
        panic!("audio transcode unexpectedly cancelled");
    };
    assert!(progress.processed_bytes > 0);
    assert!(progress.encoded_packets > 0);
    assert!(fs::metadata(&output_path).unwrap().len() > 64);

    let analyzed = adapter
        .analyze(&MediaAnalyzeRequest {
            source: MediaSource::local_file(&output_path).unwrap(),
            request_id: Some("audio-output".to_owned()),
            expected_source: None,
        })
        .unwrap();
    let (media, _) = analyzed.into_parts();
    let MediaDescriptor::Audio { streams } = media.descriptor else {
        panic!("transcoded output must remain audio");
    };
    assert_eq!(streams.len(), 1);
    assert!(matches!(
        &streams[0],
        MediaStreamDescriptor::Audio(audio)
            if audio.codec == "aac"
                && audio.sample_rate_hz.value == Some(32_000)
                && audio.channel_count.value == Some(2)
    ));
}

#[test]
fn real_libav_transcodes_two_audio_streams_into_one_m4a_without_cli() {
    let fixture = TestDirectory::new("multi-audio-transcode");
    let input_path = fixture.path.join("input.avi");
    let output_path = fixture.path.join("output.m4a");
    fs::write(&input_path, uncompressed_multi_audio_avi_fixture()).unwrap();

    let adapter = FfmpegAdapter::new().unwrap();
    adapter
        .transcode_audio(
            &AudioFileTranscodeRequest {
                input_path,
                output_path: output_path.clone(),
                audio_streams: vec![
                    AudioTranscodeRequest {
                        input_stream_index: 0,
                        decoder_name: "pcm_s16le".to_owned(),
                        encoder_name: "aac".to_owned(),
                        target_bitrate_bps: Some(64_000),
                        target_sample_rate_hz: Some(32_000),
                        target_channel_count: Some(1),
                    },
                    AudioTranscodeRequest {
                        input_stream_index: 1,
                        decoder_name: "pcm_s16le".to_owned(),
                        encoder_name: "aac".to_owned(),
                        target_bitrate_bps: Some(96_000),
                        target_sample_rate_hz: Some(48_000),
                        target_channel_count: Some(2),
                    },
                ],
            },
            |_| TranscodeControl::Continue,
        )
        .unwrap();

    let analyzed = adapter
        .analyze(&MediaAnalyzeRequest {
            source: MediaSource::local_file(&output_path).unwrap(),
            request_id: Some("multi-audio-file-output".to_owned()),
            expected_source: None,
        })
        .unwrap();
    let (media, _) = analyzed.into_parts();
    let MediaDescriptor::Audio { streams } = media.descriptor else {
        panic!("transcoded output must remain audio");
    };
    let audio_streams: Vec<_> = streams
        .iter()
        .filter_map(|stream| match stream {
            MediaStreamDescriptor::Audio(audio) => Some(audio.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(audio_streams.len(), 2);
    assert_eq!(audio_streams[0].sample_rate_hz.value, Some(32_000));
    assert_eq!(audio_streams[0].channel_count.value, Some(1));
    assert_eq!(audio_streams[1].sample_rate_hz.value, Some(48_000));
    assert_eq!(audio_streams[1].channel_count.value, Some(2));
}

fn assert_valid_bmp(path: &Path, width: i32, height: i32) -> Vec<u8> {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[0..2], b"BM");
    assert_eq!(i32::from_le_bytes(bytes[18..22].try_into().unwrap()), width);
    assert_eq!(
        i32::from_le_bytes(bytes[22..26].try_into().unwrap()),
        height
    );
    assert_eq!(u16::from_le_bytes(bytes[28..30].try_into().unwrap()), 24);
    bytes
}

fn uncompressed_avi_fixture() -> Vec<u8> {
    const WIDTH: u32 = 2;
    const HEIGHT: u32 = 2;
    const FRAME_BYTES: u32 = 16;
    let frames = [
        vec![0; FRAME_BYTES as usize],
        vec![0; FRAME_BYTES as usize],
        visible_bgr_frame(),
    ];

    let mut main_header = Vec::with_capacity(56);
    push_u32(&mut main_header, 500_000);
    push_u32(&mut main_header, FRAME_BYTES * 2);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 0x10);
    push_u32(&mut main_header, frames.len() as u32);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 1);
    push_u32(&mut main_header, FRAME_BYTES);
    push_u32(&mut main_header, WIDTH);
    push_u32(&mut main_header, HEIGHT);
    main_header.extend_from_slice(&[0; 16]);

    let mut stream_header = Vec::with_capacity(56);
    stream_header.extend_from_slice(b"vids");
    stream_header.extend_from_slice(b"DIB ");
    push_u32(&mut stream_header, 0);
    push_u16(&mut stream_header, 0);
    push_u16(&mut stream_header, 0);
    push_u32(&mut stream_header, 0);
    push_u32(&mut stream_header, 1);
    push_u32(&mut stream_header, 2);
    push_u32(&mut stream_header, 0);
    push_u32(&mut stream_header, frames.len() as u32);
    push_u32(&mut stream_header, FRAME_BYTES);
    push_u32(&mut stream_header, u32::MAX);
    push_u32(&mut stream_header, 0);
    push_i16(&mut stream_header, 0);
    push_i16(&mut stream_header, 0);
    push_i16(&mut stream_header, WIDTH as i16);
    push_i16(&mut stream_header, HEIGHT as i16);

    let mut bitmap_info = Vec::with_capacity(40);
    push_u32(&mut bitmap_info, 40);
    push_i32(&mut bitmap_info, WIDTH as i32);
    push_i32(&mut bitmap_info, HEIGHT as i32);
    push_u16(&mut bitmap_info, 1);
    push_u16(&mut bitmap_info, 24);
    push_u32(&mut bitmap_info, 0);
    push_u32(&mut bitmap_info, FRAME_BYTES);
    push_i32(&mut bitmap_info, 0);
    push_i32(&mut bitmap_info, 0);
    push_u32(&mut bitmap_info, 0);
    push_u32(&mut bitmap_info, 0);

    let stream_list = list_chunk(
        b"strl",
        [
            riff_chunk(b"strh", stream_header),
            riff_chunk(b"strf", bitmap_info),
        ]
        .concat(),
    );
    let header_list = list_chunk(
        b"hdrl",
        [riff_chunk(b"avih", main_header), stream_list].concat(),
    );

    let mut movie_data = Vec::new();
    let mut index_data = Vec::new();
    let mut offset = 4_u32;
    for frame in frames {
        let frame_size = frame.len() as u32;
        movie_data.extend_from_slice(&riff_chunk(b"00db", frame));
        index_data.extend_from_slice(b"00db");
        push_u32(&mut index_data, 0x10);
        push_u32(&mut index_data, offset);
        push_u32(&mut index_data, frame_size);
        offset += 8 + frame_size + (frame_size & 1);
    }

    let contents = [
        header_list,
        list_chunk(b"movi", movie_data),
        riff_chunk(b"idx1", index_data),
    ]
    .concat();
    riff_file(b"AVI ", contents)
}

fn uncompressed_audiovisual_avi_fixture() -> Vec<u8> {
    const WIDTH: u32 = 2;
    const HEIGHT: u32 = 2;
    const FRAME_BYTES: u32 = 16;
    const SAMPLE_RATE: u32 = 8_000;
    const SAMPLE_COUNT: u32 = 8_000;
    const AUDIO_BYTES: u32 = SAMPLE_COUNT * 2;
    let frames = [
        vec![0; FRAME_BYTES as usize],
        vec![0; FRAME_BYTES as usize],
        visible_bgr_frame(),
    ];

    let mut main_header = Vec::with_capacity(56);
    push_u32(&mut main_header, 500_000);
    push_u32(&mut main_header, SAMPLE_RATE * 2 + FRAME_BYTES * 2);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 0x10);
    push_u32(&mut main_header, frames.len() as u32);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 3);
    push_u32(&mut main_header, AUDIO_BYTES);
    push_u32(&mut main_header, WIDTH);
    push_u32(&mut main_header, HEIGHT);
    main_header.extend_from_slice(&[0; 16]);

    let mut video_header = Vec::with_capacity(56);
    video_header.extend_from_slice(b"vids");
    video_header.extend_from_slice(b"DIB ");
    push_u32(&mut video_header, 0);
    push_u16(&mut video_header, 0);
    push_u16(&mut video_header, 0);
    push_u32(&mut video_header, 0);
    push_u32(&mut video_header, 1);
    push_u32(&mut video_header, 2);
    push_u32(&mut video_header, 0);
    push_u32(&mut video_header, frames.len() as u32);
    push_u32(&mut video_header, FRAME_BYTES);
    push_u32(&mut video_header, u32::MAX);
    push_u32(&mut video_header, 0);
    push_i16(&mut video_header, 0);
    push_i16(&mut video_header, 0);
    push_i16(&mut video_header, WIDTH as i16);
    push_i16(&mut video_header, HEIGHT as i16);

    let mut bitmap_info = Vec::with_capacity(40);
    push_u32(&mut bitmap_info, 40);
    push_i32(&mut bitmap_info, WIDTH as i32);
    push_i32(&mut bitmap_info, HEIGHT as i32);
    push_u16(&mut bitmap_info, 1);
    push_u16(&mut bitmap_info, 24);
    push_u32(&mut bitmap_info, 0);
    push_u32(&mut bitmap_info, FRAME_BYTES);
    push_i32(&mut bitmap_info, 0);
    push_i32(&mut bitmap_info, 0);
    push_u32(&mut bitmap_info, 0);
    push_u32(&mut bitmap_info, 0);

    let mut audio_header = Vec::with_capacity(56);
    audio_header.extend_from_slice(b"auds");
    audio_header.extend_from_slice(&[0; 4]);
    push_u32(&mut audio_header, 0);
    push_u16(&mut audio_header, 0);
    push_u16(&mut audio_header, 0);
    push_u32(&mut audio_header, 0);
    push_u32(&mut audio_header, 2);
    push_u32(&mut audio_header, SAMPLE_RATE * 2);
    push_u32(&mut audio_header, 0);
    push_u32(&mut audio_header, SAMPLE_COUNT);
    push_u32(&mut audio_header, AUDIO_BYTES);
    push_u32(&mut audio_header, u32::MAX);
    push_u32(&mut audio_header, 2);
    audio_header.extend_from_slice(&[0; 8]);

    let mut wave_format = Vec::with_capacity(16);
    push_u16(&mut wave_format, 1);
    push_u16(&mut wave_format, 1);
    push_u32(&mut wave_format, SAMPLE_RATE);
    push_u32(&mut wave_format, SAMPLE_RATE * 2);
    push_u16(&mut wave_format, 2);
    push_u16(&mut wave_format, 16);

    let video_stream = list_chunk(
        b"strl",
        [
            riff_chunk(b"strh", video_header),
            riff_chunk(b"strf", bitmap_info),
        ]
        .concat(),
    );
    let audio_stream = list_chunk(
        b"strl",
        [
            riff_chunk(b"strh", audio_header),
            riff_chunk(b"strf", wave_format),
        ]
        .concat(),
    );
    let header_list = list_chunk(
        b"hdrl",
        [
            riff_chunk(b"avih", main_header),
            video_stream,
            audio_stream.clone(),
            audio_stream,
        ]
        .concat(),
    );

    let mut audio = Vec::with_capacity(AUDIO_BYTES as usize);
    for index in 0..SAMPLE_COUNT {
        let sample = if index % 32 < 16 {
            4_000_i16
        } else {
            -4_000_i16
        };
        audio.extend_from_slice(&sample.to_le_bytes());
    }
    let mut second_audio = Vec::with_capacity(AUDIO_BYTES as usize);
    for index in 0..SAMPLE_COUNT {
        let sample = if index % 20 < 10 {
            2_000_i16
        } else {
            -2_000_i16
        };
        second_audio.extend_from_slice(&sample.to_le_bytes());
    }
    let mut movie_data = Vec::new();
    let mut index_data = Vec::new();
    let mut offset = 4_u32;
    for (tag, data, flags) in [
        (b"00db", frames[0].clone(), 0x10),
        (b"01wb", audio, 0),
        (b"02wb", second_audio, 0),
        (b"00db", frames[1].clone(), 0x10),
        (b"00db", frames[2].clone(), 0x10),
    ] {
        let data_size = data.len() as u32;
        movie_data.extend_from_slice(&riff_chunk(tag, data));
        index_data.extend_from_slice(tag);
        push_u32(&mut index_data, flags);
        push_u32(&mut index_data, offset);
        push_u32(&mut index_data, data_size);
        offset += 8 + data_size + (data_size & 1);
    }

    riff_file(
        b"AVI ",
        [
            header_list,
            list_chunk(b"movi", movie_data),
            riff_chunk(b"idx1", index_data),
        ]
        .concat(),
    )
}

fn visible_bgr_frame() -> Vec<u8> {
    let mut frame = Vec::with_capacity(16);
    for _ in 0..2 {
        frame.extend_from_slice(&[0, 255, 0, 0, 255, 0]);
        frame.extend_from_slice(&[0, 0]);
    }
    frame
}

fn pcm_wav_fixture() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 8_000;
    const SAMPLE_COUNT: u32 = 4_000;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    let data_size = SAMPLE_COUNT * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
    let mut bytes = Vec::with_capacity((44 + data_size) as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&(CHANNELS * (BITS_PER_SAMPLE / 8)).to_le_bytes());
    bytes.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for index in 0..SAMPLE_COUNT {
        let sample = if index % 16 < 8 {
            4_000_i16
        } else {
            -4_000_i16
        };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn uncompressed_multi_audio_avi_fixture() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 8_000;
    const SAMPLE_COUNT: u32 = 4_000;
    const AUDIO_BYTES: u32 = SAMPLE_COUNT * 2;

    let mut main_header = Vec::with_capacity(56);
    push_u32(&mut main_header, 500_000);
    push_u32(&mut main_header, SAMPLE_RATE * 4);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 0x10);
    push_u32(&mut main_header, 1);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 2);
    push_u32(&mut main_header, AUDIO_BYTES);
    push_u32(&mut main_header, 0);
    push_u32(&mut main_header, 0);
    main_header.extend_from_slice(&[0; 16]);

    let mut audio_header = Vec::with_capacity(56);
    audio_header.extend_from_slice(b"auds");
    audio_header.extend_from_slice(&[0; 4]);
    push_u32(&mut audio_header, 0);
    push_u16(&mut audio_header, 0);
    push_u16(&mut audio_header, 0);
    push_u32(&mut audio_header, 0);
    push_u32(&mut audio_header, 2);
    push_u32(&mut audio_header, SAMPLE_RATE * 2);
    push_u32(&mut audio_header, 0);
    push_u32(&mut audio_header, SAMPLE_COUNT);
    push_u32(&mut audio_header, AUDIO_BYTES);
    push_u32(&mut audio_header, u32::MAX);
    push_u32(&mut audio_header, 2);
    audio_header.extend_from_slice(&[0; 8]);

    let mut wave_format = Vec::with_capacity(16);
    push_u16(&mut wave_format, 1);
    push_u16(&mut wave_format, 1);
    push_u32(&mut wave_format, SAMPLE_RATE);
    push_u32(&mut wave_format, SAMPLE_RATE * 2);
    push_u16(&mut wave_format, 2);
    push_u16(&mut wave_format, 16);
    let audio_stream = list_chunk(
        b"strl",
        [
            riff_chunk(b"strh", audio_header),
            riff_chunk(b"strf", wave_format),
        ]
        .concat(),
    );
    let header_list = list_chunk(
        b"hdrl",
        [
            riff_chunk(b"avih", main_header),
            audio_stream.clone(),
            audio_stream,
        ]
        .concat(),
    );

    let audio = |period: u32, amplitude: i16| {
        let mut bytes = Vec::with_capacity(AUDIO_BYTES as usize);
        for index in 0..SAMPLE_COUNT {
            let sample = if index % period < period / 2 {
                amplitude
            } else {
                -amplitude
            };
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    };
    let chunks = [(b"00wb", audio(32, 4_000)), (b"01wb", audio(20, 2_000))];
    let mut movie_data = Vec::new();
    let mut index_data = Vec::new();
    let mut offset = 4_u32;
    for (tag, data) in chunks {
        movie_data.extend_from_slice(&riff_chunk(tag, data));
        index_data.extend_from_slice(tag);
        push_u32(&mut index_data, 0);
        push_u32(&mut index_data, offset);
        push_u32(&mut index_data, AUDIO_BYTES);
        offset += 8 + AUDIO_BYTES;
    }
    riff_file(
        b"AVI ",
        [
            header_list,
            list_chunk(b"movi", movie_data),
            riff_chunk(b"idx1", index_data),
        ]
        .concat(),
    )
}

fn riff_file(kind: &[u8; 4], contents: Vec<u8>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(12 + contents.len());
    bytes.extend_from_slice(b"RIFF");
    push_u32(&mut bytes, (4 + contents.len()) as u32);
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(&contents);
    bytes
}

fn list_chunk(kind: &[u8; 4], contents: Vec<u8>) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + contents.len());
    data.extend_from_slice(kind);
    data.extend_from_slice(&contents);
    riff_chunk(b"LIST", data)
}

fn riff_chunk(tag: &[u8; 4], mut data: Vec<u8>) -> Vec<u8> {
    let data_len = data.len();
    let mut bytes = Vec::with_capacity(8 + data_len + (data_len & 1));
    bytes.extend_from_slice(tag);
    push_u32(&mut bytes, data_len as u32);
    bytes.append(&mut data);
    if data_len & 1 != 0 {
        bytes.push(0);
    }
    bytes
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("framelean-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
