//! Importing a sound: a file in any format that is read (MP3, FLAC, OGG Vorbis, WAV, AAC) becomes a stored sound. A video
//! file (MP4, MOV, MKV, WebM) gives its sound: the first audio track is kept and the picture is never read.
//!
//! The file is decoded a packet at a time and written to the stored format as it comes, so that a very long sound (a
//! whole video's audio) never has to fit in memory. Sounds that are not at 48 kHz are converted once, here; a sound with
//! more than two channels keeps its first two (left and right).

use crate::wav::WavWriter;
use crate::{ENGINE_RATE, MAX_CHANNELS};
use omu_dsp::resample::StreamResampler;
use omu_types::SoundInfo;
use std::fs::File;
use std::path::Path;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Why a file was not imported. `Unsupported` is a file this program cannot read by itself (the FFmpeg module may be
/// able to); `Failed` is anything else (a damaged file, a full disk).
#[derive(Debug)]
pub enum ImportError {
    Unsupported(String),
    Failed(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Unsupported(s) | ImportError::Failed(s) => f.write_str(s),
        }
    }
}

impl From<String> for ImportError {
    fn from(s: String) -> ImportError {
        ImportError::Failed(s)
    }
}

fn describe(e: Error) -> String {
    format!("{e}")
}

/// Frames converted at once.
const CHUNK_FRAMES: usize = 4096;

fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Reads `source` and writes the stored sound `id` to `destination`.
pub fn import_file(
    source: &Path,
    destination: &Path,
    id: &str,
    name: &str,
) -> Result<SoundInfo, ImportError> {
    import_file_with_progress(source, destination, id, name, &mut |_, _| {})
}

/// The same, and `progress(read, total)` is called as the file is read: the frames of the file read so far, and how many it has
/// (0 when the file does not say).
pub fn import_file_with_progress(
    source: &Path,
    destination: &Path,
    id: &str,
    name: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<SoundInfo, ImportError> {
    let file = File::open(source).map_err(|e| format!("cannot open {}: {e}", source.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| {
            ImportError::Unsupported(
                "this file is not a sound or a video that can be read (WAV, MP3, FLAC, OGG, AAC, and the sound of MP4, MOV, MKV and WebM videos are)".into(),
            )
        })?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| {
        ImportError::Unsupported("there is no sound in this file that can be read".into())
    })?;
    let track_id = track.id;
    let total_frames = track.num_frames.unwrap_or(0);
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| {
            ImportError::Unsupported("there is no sound in this file that can be read".into())
        })?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|_| {
            ImportError::Unsupported(
                "the sound in this file is not in a format that can be read (a video's sound in Opus, for example)".into(),
            )
        })?;

    if let Some(dir) = destination.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let result = convert(
        &mut format,
        &mut decoder,
        track_id,
        destination,
        total_frames,
        progress,
    );
    match result {
        Ok((channels, frames)) => {
            let bytes = std::fs::metadata(destination).map(|m| m.len()).unwrap_or(0);
            if frames == 0 {
                let _ = std::fs::remove_file(destination);
                return Err(ImportError::Failed("this file has no sound in it".into()));
            }
            Ok(SoundInfo {
                id: id.to_owned(),
                name: name.to_owned(),
                channels: channels as u8,
                frames,
                bytes,
                compressed: false,
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(destination);
            Err(ImportError::Failed(e))
        }
    }
}

fn convert(
    format: &mut Box<dyn symphonia::core::formats::FormatReader>,
    decoder: &mut Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    track_id: u32,
    destination: &Path,
    total_frames: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(usize, u64), String> {
    let mut read_frames = 0u64;
    let mut writer: Option<WavWriter> = None;
    let mut channels = 0usize;
    let mut resampler: Option<(StreamResampler, f64)> = None;
    let mut decoded: Vec<f32> = Vec::new();
    // the sound at the stored channel count, waiting to be converted
    let mut pending: Vec<f32> = Vec::new();
    let mut converted: Vec<f32> = Vec::new();
    let mut samples: Vec<i16> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(describe(e)),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(b) => b,
            // a damaged packet is skipped (a click in a sound is better than refusing it)
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(describe(e)),
        };
        let (rate, source_channels) = (buffer.spec().rate(), buffer.spec().channels().count());
        if source_channels == 0 || rate == 0 {
            continue;
        }
        if writer.is_none() {
            channels = source_channels.min(MAX_CHANNELS);
            writer = Some(WavWriter::create(destination, channels).map_err(|e| e.to_string())?);
            if rate != ENGINE_RATE {
                resampler = Some((
                    StreamResampler::new(channels),
                    rate as f64 / ENGINE_RATE as f64,
                ));
            }
        }
        read_frames += buffer.frames() as u64;
        progress(read_frames, total_frames);
        decoded.resize(buffer.samples_interleaved(), 0.0);
        buffer.copy_to_slice_interleaved(&mut decoded);
        // keep the first two channels
        for frame in decoded.chunks_exact(source_channels) {
            pending.extend_from_slice(&frame[..channels]);
        }
        let w = writer.as_mut().expect("the writer exists");
        write_pending(
            w,
            &mut pending,
            &mut resampler,
            channels,
            false,
            &mut converted,
            &mut samples,
        )?;
    }
    let Some(mut w) = writer else {
        return Err("this file has no sound in it".into());
    };
    write_pending(
        &mut w,
        &mut pending,
        &mut resampler,
        channels,
        true,
        &mut converted,
        &mut samples,
    )?;
    let frames = w.frames();
    w.finish().map_err(|e| e.to_string())?;
    Ok((channels, frames))
}

/// Converts what is pending to 48 kHz and writes it. With `last`, the tail is flushed.
fn write_pending(
    writer: &mut WavWriter,
    pending: &mut Vec<f32>,
    resampler: &mut Option<(StreamResampler, f64)>,
    channels: usize,
    last: bool,
    converted: &mut Vec<f32>,
    samples: &mut Vec<i16>,
) -> Result<(), String> {
    let Some((rs, step)) = resampler else {
        // already 48 kHz
        samples.clear();
        samples.extend(pending.iter().map(|x| to_i16(*x)));
        pending.clear();
        return writer.write(samples).map_err(|e| e.to_string());
    };
    if last {
        // the interpolator looks two frames ahead: give it silence to read the end with
        pending.extend(std::iter::repeat_n(0.0, 4 * channels));
    }
    converted.resize(CHUNK_FRAMES * channels, 0.0);
    loop {
        let (used, made) = rs.process(pending, converted, *step);
        if made > 0 {
            samples.clear();
            samples.extend(converted[..made * channels].iter().map(|x| to_i16(*x)));
            writer.write(samples).map_err(|e| e.to_string())?;
        }
        pending.drain(..used * channels);
        if made < CHUNK_FRAMES {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::{read_info, read_samples};

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("omu-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A WAV file of any rate, 16-bit, written by hand (the stored-format writer only does 48 kHz).
    fn wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
        let data = (samples.len() * 2) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn the_progress_of_an_import_goes_up_to_the_whole_file() {
        let dir = temp("progress");
        let source = dir.join("in.wav");
        wav(&source, 44_100, 1, &tone(44_100, 2.0, 440.0));
        let mut seen: Vec<(u64, u64)> = vec![];
        let info = import_file_with_progress(
            &source,
            &dir.join("out.wav"),
            "id",
            "name",
            &mut |read, total| seen.push((read, total)),
        )
        .unwrap();
        assert!(info.frames > 0);
        assert!(seen.len() > 1, "reported more than once");
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "never goes back");
        let (read, total) = *seen.last().unwrap();
        assert_eq!(total, 88_200, "the file says how long it is");
        assert_eq!(read, total, "ends with the whole file read");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tone(rate: u32, seconds: f32, hz: f32) -> Vec<i16> {
        (0..(rate as f32 * seconds) as usize)
            .map(|i| {
                ((i as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * 12_000.0) as i16
            })
            .collect()
    }

    fn zero_crossings(x: &[i16]) -> usize {
        x.windows(2).filter(|w| w[0] <= 0 && w[1] > 0).count()
    }

    #[test]
    fn a_sound_already_at_48_khz_is_stored_as_it_is() {
        let dir = temp("same");
        let samples: Vec<i16> = tone(48_000, 0.5, 440.0)
            .iter()
            .flat_map(|s| [*s, s / 2])
            .collect();
        wav(&dir.join("in.wav"), 48_000, 2, &samples);
        let info = import_file(&dir.join("in.wav"), &dir.join("out.wav"), "id1", "in.wav").unwrap();
        assert_eq!(
            (
                info.channels,
                info.frames,
                info.id.as_str(),
                info.name.as_str()
            ),
            (2, 24_000, "id1", "in.wav")
        );
        assert_eq!(info.bytes, 44 + 24_000 * 4);
        let mut f = File::open(dir.join("out.wav")).unwrap();
        let stored = read_info(&mut f).unwrap();
        let mut back = vec![0i16; 48_000];
        read_samples(&mut f, &stored, 0, &mut back).unwrap();
        // within the rounding of the conversion to float and back
        assert!(back
            .iter()
            .zip(&samples)
            .all(|(a, b)| (*a as i32 - *b as i32).abs() <= 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sound_at_44100_hz_is_converted_to_48000_with_the_same_pitch_and_length() {
        let dir = temp("rate");
        wav(&dir.join("in.wav"), 44_100, 1, &tone(44_100, 1.0, 1000.0));
        let info = import_file(&dir.join("in.wav"), &dir.join("out.wav"), "id", "tone").unwrap();
        assert_eq!(info.channels, 1);
        assert!(
            (info.frames as i64 - 48_000).abs() <= 8,
            "{} frames",
            info.frames
        );
        let mut f = File::open(dir.join("out.wav")).unwrap();
        let stored = read_info(&mut f).unwrap();
        let mut back = vec![0i16; info.frames as usize];
        read_samples(&mut f, &stored, 0, &mut back).unwrap();
        // 1000 Hz for one second: 1000 periods, wherever they are counted
        let periods = zero_crossings(&back);
        assert!((periods as i64 - 1000).abs() <= 2, "{periods} periods");
        let peak = back.iter().map(|s| s.abs()).max().unwrap();
        assert!((peak as i32 - 12_000).abs() < 400, "peak {peak}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn more_than_two_channels_keep_the_first_two_and_a_bad_file_is_refused_with_nothing_left() {
        let dir = temp("bad");
        let quad: Vec<i16> = (0..4000).map(|i| [1000, -1000, 5, 6][i % 4]).collect();
        wav(&dir.join("quad.wav"), 48_000, 4, &quad);
        let info = import_file(&dir.join("quad.wav"), &dir.join("q.wav"), "q", "quad").unwrap();
        assert_eq!((info.channels, info.frames), (2, 1000));
        let mut f = File::open(dir.join("q.wav")).unwrap();
        let stored = read_info(&mut f).unwrap();
        let mut s = [0i16; 4];
        read_samples(&mut f, &stored, 0, &mut s).unwrap();
        assert!(s[0] > 900 && s[1] < -900, "{s:?}");

        std::fs::write(dir.join("text.mp3"), b"this is only text, not a sound").unwrap();
        let err = import_file(&dir.join("text.mp3"), &dir.join("t.wav"), "t", "text").unwrap_err();
        assert!(err.to_string().contains("not a sound"), "{err}");
        assert!(!dir.join("t.wav").exists(), "no half-written file");
        assert!(import_file(&dir.join("absent.wav"), &dir.join("a.wav"), "a", "a").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn media(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("media")
            .join(name)
    }

    /// Imports a file of tests/media and returns the stored samples (left channel) and the info.
    fn import_media(name: &str) -> Result<(SoundInfo, Vec<i16>), String> {
        let dir = temp(&format!("media-{name}"));
        let out = dir.join("out.wav");
        let info = import_file(&media(name), &out, "m", name).map_err(|e| e.to_string())?;
        let mut f = File::open(&out).unwrap();
        let stored = read_info(&mut f).unwrap();
        let mut back = vec![0i16; (info.frames * info.channels as u64) as usize];
        read_samples(&mut f, &stored, 0, &mut back).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Ok((info, back))
    }

    fn loud_and_one_second(name: &str) {
        let (info, samples) = import_media(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            (info.frames as i64 - 48_000).abs() < 4_000,
            "{name}: about one second at 48 kHz: {} frames",
            info.frames
        );
        let ch = info.channels as usize;
        let middle: Vec<f32> = samples
            .iter()
            .step_by(ch)
            .skip(10_000)
            .take(20_000)
            .map(|s| *s as f32 / 32768.0)
            .collect();
        let rms = (middle.iter().map(|x| x * x).sum::<f32>() / middle.len() as f32).sqrt();
        assert!(rms > 0.05, "{name}: the tone is there (rms {rms})");
        // the pitch is kept: about 440 zero crossings upward per second
        let rising = middle
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        let hz = rising as f32 / (20_000.0 / 48_000.0);
        assert!((hz - 440.0).abs() < 25.0, "{name}: {hz} Hz");
    }

    #[test]
    fn the_sound_of_a_video_is_kept_and_the_picture_is_ignored() {
        // MP4 with AAC (the common case: phones, screen recorders), MKV with Vorbis
        loud_and_one_second("tone-aac.mp4");
        loud_and_one_second("tone-vorbis.mkv");
    }

    #[test]
    fn an_aac_sound_file_is_read_too() {
        loud_and_one_second("tone.m4a");
    }

    #[test]
    fn a_video_whose_sound_is_in_a_format_that_cannot_be_read_says_so() {
        let e = import_media("tone-opus.webm").unwrap_err();
        assert!(
            e.contains("format that can be read") || e.contains("no sound"),
            "{e}"
        );
    }
}
