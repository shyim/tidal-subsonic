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
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;

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

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            symphonia::core::meta::MetadataOptions::default(),
        )
        .map_err(|e| format!("probe failed: {}", e))?;

    // Pick the default audio track and pull out its audio codec params.
    let track = format
        .default_track(symphonia::core::formats::TrackType::Audio)
        .ok_or_else(|| "no default audio track".to_string())?;
    let track_id = track.id;
    let audio_params = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err("track has no audio codec params".to_string()),
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &Default::default())
        .map_err(|e| format!("decoder init failed: {}", e))?;

    let mut sample_rate = audio_params.sample_rate.unwrap_or(44100);
    let mut channels = audio_params.channels.as_ref().map(|c| c.count()).unwrap_or(2);
    let mut interleaved: Vec<i16> = Vec::new();
    // Per-packet scratch: copy_to_vec_interleaved *resizes* its target to the
    // current buffer, so we decode into `frame` then extend the accumulator.
    let mut frame: Vec<i16> = Vec::new();

    // 0.6: next_packet() returns Ok(None) at clean end of stream.
    while let Some(packet) = format
        .next_packet()
        .map_err(|e| format!("read packet: {}", e))?
    {
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                sample_rate = decoded.spec().rate();
                channels = decoded.spec().channels().count();
                // Convert whatever sample format the decoder produced into
                // interleaved i16, then append to the running output.
                decoded.copy_to_vec_interleaved(&mut frame);
                interleaved.extend_from_slice(&frame);
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
