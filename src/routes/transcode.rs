//! Transcode TIDAL's AAC-LC-in-fMP4 audio to MP3, in-process (pure Rust).
//!
//! Some clients — notably the Garmin YuMusic app — hardcode `format=mp3` and
//! declare `Media.ENCODING_MP3`, so they need actual MP3 bytes, not the
//! AAC-in-MP4 the proxy normally serves. We decode with `symphonia` (isomp4
//! demuxer + AAC decoder) and re-encode with LAME via `mp3lame-encoder`.
//!
//! NOTE: symphonia decodes AAC-LC (`mp4a.40.2`, TIDAL "HIGH") but not HE-AAC
//! (`mp4a.40.5`, TIDAL "LOW"). Callers MUST resolve the track at HIGH quality
//! before handing the bytes here.

use std::io::Cursor;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded PCM ready for MP3 encoding.
struct Pcm {
    sample_rate: u32,
    channels: usize,
    /// Interleaved i16 samples (L,R,L,R,… for stereo).
    interleaved: Vec<i16>,
}

/// Decode an fMP4/M4A byte buffer containing AAC-LC into interleaved i16 PCM.
fn decode_aac_mp4(data: Vec<u8>) -> Result<Pcm, String> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(data)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    hint.mime_type("audio/mp4");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe failed: {}", e))?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| "no default track".to_string())?
        .clone();
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder init failed: {}", e))?;

    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2);
    let mut interleaved: Vec<i16> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Clean end of stream.
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("read packet: {}", e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                sample_rate = decoded.spec().rate;
                channels = decoded.spec().channels.count();
                append_interleaved(&decoded, &mut interleaved);
            }
            // Decoder resync errors are recoverable — skip the packet.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode: {}", e)),
        }
    }

    if interleaved.is_empty() {
        return Err("no audio decoded".to_string());
    }

    Ok(Pcm {
        sample_rate,
        channels,
        interleaved,
    })
}

/// Append a decoded audio buffer to the interleaved i16 output, converting from
/// whatever sample format symphonia produced.
fn append_interleaved(decoded: &AudioBufferRef, out: &mut Vec<i16>) {
    macro_rules! interleave {
        ($buf:expr, $conv:expr) => {{
            let buf = $buf;
            let ch = buf.spec().channels.count();
            let frames = buf.frames();
            out.reserve(frames * ch);
            for f in 0..frames {
                for c in 0..ch {
                    out.push($conv(buf.chan(c)[f]));
                }
            }
        }};
    }
    match decoded {
        AudioBufferRef::S16(b) => interleave!(b.as_ref(), |s: i16| s),
        AudioBufferRef::S32(b) => interleave!(b.as_ref(), |s: i32| (s >> 16) as i16),
        AudioBufferRef::F32(b) => interleave!(b.as_ref(), |s: f32| {
            (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
        }),
        AudioBufferRef::F64(b) => interleave!(b.as_ref(), |s: f64| {
            (s.clamp(-1.0, 1.0) * i16::MAX as f64) as i16
        }),
        AudioBufferRef::S24(b) => interleave!(b.as_ref(), |s: symphonia::core::sample::i24| {
            (s.inner() >> 8) as i16
        }),
        AudioBufferRef::U8(b) => {
            interleave!(b.as_ref(), |s: u8| (s as i16 - 128) << 8)
        }
        AudioBufferRef::U16(b) => interleave!(b.as_ref(), |s: u16| (s as i32 - 32768) as i16),
        AudioBufferRef::U24(b) => interleave!(b.as_ref(), |s: symphonia::core::sample::u24| {
            ((s.inner() as i32 >> 8) - 32768) as i16
        }),
        AudioBufferRef::U32(b) => {
            interleave!(b.as_ref(), |s: u32| ((s >> 16) as i32 - 32768) as i16)
        }
        AudioBufferRef::S8(b) => interleave!(b.as_ref(), |s: i8| (s as i16) << 8),
    }
}

/// Encode interleaved i16 PCM to MP3 at the target bitrate (kbps).
fn encode_mp3(pcm: Pcm, bitrate_kbps: u32) -> Result<Vec<u8>, String> {
    use mp3lame_encoder::{Builder, FlushNoGap, MonoPcm, DualPcm};

    let mut builder = Builder::new().ok_or("lame builder init failed")?;
    builder
        .set_num_channels(pcm.channels.max(1) as u8)
        .map_err(|e| format!("set channels: {:?}", e))?;
    builder
        .set_sample_rate(pcm.sample_rate)
        .map_err(|e| format!("set sample rate: {:?}", e))?;
    let br = nearest_lame_bitrate(bitrate_kbps);
    builder
        .set_brate(br)
        .map_err(|e| format!("set bitrate: {:?}", e))?;

    let mut encoder = builder
        .build()
        .map_err(|e| format!("lame build: {:?}", e))?;

    // Worst-case MP3 size bound recommended by LAME.
    let mut out: Vec<u8> = Vec::with_capacity(pcm.interleaved.len() + 7200);

    if pcm.channels >= 2 {
        // De-interleave into left/right planes.
        let frames = pcm.interleaved.len() / pcm.channels;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for f in 0..frames {
            left.push(pcm.interleaved[f * pcm.channels]);
            right.push(pcm.interleaved[f * pcm.channels + 1]);
        }
        let input = DualPcm {
            left: &left,
            right: &right,
        };
        let buf = std::mem::take(&mut out);
        let mut spare = buf;
        spare.reserve(mp3lame_encoder::max_required_buffer_size(frames));
        let encoded = encoder
            .encode(input, spare.spare_capacity_mut())
            .map_err(|e| format!("encode: {:?}", e))?;
        unsafe { spare.set_len(encoded) };
        out = spare;
    } else {
        let input = MonoPcm(&pcm.interleaved);
        out.reserve(mp3lame_encoder::max_required_buffer_size(pcm.interleaved.len()));
        let encoded = encoder
            .encode(input, out.spare_capacity_mut())
            .map_err(|e| format!("encode: {:?}", e))?;
        unsafe { out.set_len(encoded) };
    }

    // Flush the encoder's final frames.
    let prev = out.len();
    out.reserve(7200);
    let flushed = encoder
        .flush::<FlushNoGap>(out.spare_capacity_mut())
        .map_err(|e| format!("flush: {:?}", e))?;
    unsafe { out.set_len(prev + flushed) };

    Ok(out)
}

/// Snap a requested bitrate to a value LAME accepts.
fn nearest_lame_bitrate(kbps: u32) -> mp3lame_encoder::Bitrate {
    use mp3lame_encoder::Bitrate::*;
    match kbps {
        0..=96 => Kbps96,
        97..=128 => Kbps128,
        129..=160 => Kbps160,
        161..=192 => Kbps192,
        193..=256 => Kbps256,
        _ => Kbps320,
    }
}

/// Full pipeline: AAC-LC-in-fMP4 bytes → MP3 bytes at the given bitrate.
/// Runs the CPU-bound decode+encode on a blocking thread.
pub(crate) async fn aac_mp4_to_mp3(data: Vec<u8>, bitrate_kbps: u32) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || {
        let pcm = decode_aac_mp4(data)?;
        encode_mp3(pcm, bitrate_kbps)
    })
    .await
    .map_err(|e| format!("transcode task panicked: {}", e))?
}
