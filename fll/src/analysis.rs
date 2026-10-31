use crate::{
    Operation,
    backends::libav::{Input, streams},
    source,
};
use fll_contracts::*;
use rusty_ffmpeg::ffi as av;
use std::{ffi::CStr, path::Path, ptr};
pub const ANALYZER: &str = "libav-probe-2";
unsafe fn name(p: *const std::ffi::c_char) -> Fact<String> {
    if p.is_null() {
        Fact::unknown("not reported")
    } else {
        let name = unsafe { CStr::from_ptr(p) }.to_string_lossy();
        if matches!(name.as_ref(), "unknown" | "unspecified") {
            Fact::unknown("not reported")
        } else {
            Fact::known(name.into_owned())
        }
    }
}
fn rational(r: av::AVRational) -> Fact<Rational> {
    if r.num > 0 && r.den > 0 {
        Fact::known(Rational {
            num: r.num,
            den: r.den,
        })
    } else {
        Fact::unknown("not reported")
    }
}
fn positive(n: i32) -> Fact<i32> {
    if n > 0 {
        Fact::known(n)
    } else {
        Fact::unknown("not reported")
    }
}
unsafe fn timestamp(n: i64, time: av::AVRational) -> Fact<i64> {
    if n == av::AV_NOPTS_VALUE || time.den <= 0 {
        Fact::unknown("not reported")
    } else {
        Fact::known(unsafe {
            av::av_rescale_q(
                n,
                time,
                av::AVRational {
                    num: 1,
                    den: 1_000_000,
                },
            )
        })
    }
}
pub(crate) fn inspect(input: &Input, stamp: SourceStamp) -> MediaFacts {
    unsafe {
        let context = &*input.0;
        let tracks = streams(input)
            .iter()
            .enumerate()
            .map(|(index, &s)| {
                let stream = &*s;
                let p = &*stream.codecpar;
                let kind = match p.codec_type {
                    av::AVMEDIA_TYPE_VIDEO => "video",
                    av::AVMEDIA_TYPE_AUDIO => "audio",
                    av::AVMEDIA_TYPE_SUBTITLE => "subtitle",
                    av::AVMEDIA_TYPE_ATTACHMENT => "attachment",
                    av::AVMEDIA_TYPE_DATA => "data",
                    _ => "unknown",
                }
                .to_string();
                let video = if p.codec_type == av::AVMEDIA_TYPE_VIDEO {
                    let side = av::av_packet_side_data_get(
                        p.coded_side_data,
                        p.nb_coded_side_data,
                        av::AV_PKT_DATA_DISPLAYMATRIX,
                    );
                    let matrix = if side.is_null() {
                        Fact::known(Vec::new())
                    } else if (*side).size >= 36 {
                        Fact::known(
                            std::slice::from_raw_parts((*side).data.cast::<i32>(), 9).to_vec(),
                        )
                    } else {
                        Fact::unknown("invalid display matrix")
                    };
                    let rotation =
                        av::av_dict_get(stream.metadata, c"rotate".as_ptr(), ptr::null(), 0);
                    let descriptor = av::av_pix_fmt_desc_get(p.format);
                    Some(VideoFacts {
                        width: positive(p.width),
                        height: positive(p.height),
                        pixel_format: name(av::av_get_pix_fmt_name(p.format)),
                        bit_depth: if descriptor.is_null() {
                            Fact::unknown("pixel format unknown")
                        } else {
                            Fact::known((*descriptor).comp[0].depth)
                        },
                        sample_aspect_ratio: rational(av::av_guess_sample_aspect_ratio(
                            input.0,
                            s,
                            ptr::null_mut(),
                        )),
                        frame_rate: rational(av::av_guess_frame_rate(input.0, s, ptr::null_mut())),
                        display_matrix: matrix,
                        rotate_tag: if rotation.is_null() {
                            None
                        } else {
                            Some(
                                CStr::from_ptr((*rotation).value)
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                        },
                        color: ColorFacts {
                            primaries: name(av::av_color_primaries_name(p.color_primaries)),
                            transfer: name(av::av_color_transfer_name(p.color_trc)),
                            space: name(av::av_color_space_name(p.color_space)),
                            range: name(av::av_color_range_name(p.color_range)),
                        },
                    })
                } else {
                    None
                };
                TrackFacts {
                    index: index as u32,
                    kind,
                    codec: name(av::avcodec_get_name(p.codec_id)),
                    profile: name(av::avcodec_profile_name(p.codec_id, p.profile)),
                    time_base: rational(stream.time_base),
                    start_time_us: timestamp(stream.start_time, stream.time_base),
                    duration_us: timestamp(stream.duration, stream.time_base),
                    video,
                    audio: if p.codec_type == av::AVMEDIA_TYPE_AUDIO {
                        Some(AudioFacts {
                            sample_rate: positive(p.sample_rate),
                            channels: positive(p.ch_layout.nb_channels),
                        })
                    } else {
                        None
                    },
                    language: {
                        let entry =
                            av::av_dict_get(stream.metadata, c"language".as_ptr(), ptr::null(), 0);
                        if entry.is_null() {
                            None
                        } else {
                            let value = CStr::from_ptr((*entry).value)
                                .to_string_lossy()
                                .into_owned();
                            if value == "und" || value.is_empty() {
                                None
                            } else {
                                Some(value)
                            }
                        }
                    },
                    disposition: stream.disposition,
                    side_data: (0..p.nb_coded_side_data)
                        .map(|i| {
                            let data = &*p.coded_side_data.add(i as usize);
                            let value = av::av_packet_side_data_name(data.type_);
                            if value.is_null() {
                                format!("unknown:{}", data.type_)
                            } else {
                                CStr::from_ptr(value).to_string_lossy().into_owned()
                            }
                        })
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        let mut facts = MediaFacts {
            schema_version: FACTS_VERSION,
            analyzer_version: ANALYZER.into(),
            source_stamp: stamp,
            read_status: "probed".into(),
            container: CStr::from_ptr((*context.iformat).name)
                .to_string_lossy()
                .into_owned(),
            duration_us: if context.duration > 0 {
                Fact::known(context.duration)
            } else {
                Fact::unknown("duration not reported")
            },
            tracks,
            chapter_count: context.nb_chapters,
            program_count: context.nb_programs,
            warnings: vec![],
            facts_digest: String::new(),
        };
        if facts.duration_us.get().is_none() {
            facts.read_status = "partial".into();
            facts.warnings.push("duration_unknown".into());
        }
        facts.facts_digest = digest(&facts);
        facts
    }
}
pub(crate) fn analyze(
    path: &str,
    cached: Option<&MediaFacts>,
    op: &Operation,
) -> Result<MediaFacts> {
    op.check()?;
    let before = source::stamp(Path::new(path))?;
    if let Some(c) = cached
        && c.source_stamp == before
        && c.schema_version == FACTS_VERSION
        && c.analyzer_version == ANALYZER
    {
        let mut verified = c.clone();
        verified.facts_digest.clear();
        if digest(&verified) == c.facts_digest {
            return Ok(c.clone());
        }
    }
    let input =
        Input::open(path, op).map_err(|e| MediaError::new("media_unreadable", "analysis", e))?;
    op.check()?;
    let facts = inspect(&input, before.clone());
    if source::stamp(Path::new(path))? != before {
        return Err(MediaError::new(
            "source_changed",
            "analysis",
            "分析期间源文件变化",
        ));
    }
    Ok(facts)
}
