//! Stored sounds in either shape they can have, behind one reader.
//!
//! * **WAV** (see [`super::wav`]): the plain shape every imported sound starts in. The samples are the bytes of the
//!   file, so any position is a position in the file.
//! * **Segmented FLAC** (`.omuf`): the same samples compressed without loss, for long sounds. It is a small header and a
//!   table, then independent FLAC streams of [`SEGMENT_FRAMES`] frames each (the length of the segments the reader
//!   works in), so that any segment can be read without reading what is before it. Nothing in it is lossy: what comes
//!   back is exactly the samples that went in.
//!
//! ```text
//! "OMUF"  version(1)  channels(1)  reserved(2)  frames(8)  segment_frames(4)  segments(4)   = 24 bytes
//! per segment: offset(8) length(4)
//! the FLAC streams
//! ```
//! All numbers are little-endian.

use super::wav::{self, WavInfo};
use super::{ENGINE_RATE, MAX_CHANNELS, SEGMENT_SECONDS};
use std::fs::File;
use std::io::{self, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"OMUF";
const VERSION: u8 = 1;
const HEADER_BYTES: u64 = 24;
const ENTRY_BYTES: u64 = 12;

/// Frames in a segment of a compressed sound (the segment the reader works in).
pub const SEGMENT_FRAMES: u64 = SEGMENT_SECONDS * ENGINE_RATE as u64;

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// A stored sound, open for reading.
pub enum Stored {
    Wav { file: File, info: WavInfo },
    Flac(Box<FlacStored>),
}

pub struct FlacStored {
    file: File,
    channels: usize,
    frames: u64,
    segment_frames: u64,
    /// (offset, length) of each segment.
    index: Vec<(u64, u32)>,
    /// The segment decoded last: reading goes forward, so one is enough.
    cache: Option<(usize, Vec<i16>)>,
}

impl Stored {
    /// Opens a stored sound of either shape.
    pub fn open(path: &Path) -> io::Result<Stored> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic == MAGIC {
            Ok(Stored::Flac(Box::new(FlacStored::read_header(file)?)))
        } else {
            let info = wav::read_info(&mut file)?;
            Ok(Stored::Wav { file, info })
        }
    }

    pub fn channels(&self) -> usize {
        match self {
            Stored::Wav { info, .. } => info.channels,
            Stored::Flac(f) => f.channels,
        }
    }

    pub fn frames(&self) -> u64 {
        match self {
            Stored::Wav { info, .. } => info.frames,
            Stored::Flac(f) => f.frames,
        }
    }

    /// Reads up to `out.len()` samples (interleaved) from `start_frame`; returns how many samples were read (fewer only
    /// at the end of the sound).
    pub fn read_samples(&mut self, start_frame: u64, out: &mut [i16]) -> io::Result<usize> {
        match self {
            Stored::Wav { file, info } => wav::read_samples(file, info, start_frame, out),
            Stored::Flac(f) => f.read_samples(start_frame, out),
        }
    }
}

impl FlacStored {
    fn read_header(mut file: File) -> io::Result<FlacStored> {
        let mut rest = [0u8; (HEADER_BYTES - 4) as usize];
        file.read_exact(&mut rest)?;
        if rest[0] != VERSION {
            return Err(bad("this compressed sound was made by a newer version"));
        }
        let channels = rest[1] as usize;
        let frames = u64::from_le_bytes(rest[4..12].try_into().expect("8 bytes"));
        let segment_frames = u32::from_le_bytes(rest[12..16].try_into().expect("4 bytes")) as u64;
        let segments = u32::from_le_bytes(rest[16..20].try_into().expect("4 bytes")) as usize;
        if !(1..=MAX_CHANNELS).contains(&channels)
            || segment_frames == 0
            || segments as u64 != frames.div_ceil(segment_frames)
        {
            return Err(bad("the compressed sound's header is not valid"));
        }
        let mut table = vec![0u8; segments * ENTRY_BYTES as usize];
        file.read_exact(&mut table)?;
        let index = table
            .as_chunks::<{ ENTRY_BYTES as usize }>()
            .0
            .iter()
            .map(|e| {
                (
                    u64::from_le_bytes(e[0..8].try_into().expect("8 bytes")),
                    u32::from_le_bytes(e[8..12].try_into().expect("4 bytes")),
                )
            })
            .collect();
        Ok(FlacStored {
            file,
            channels,
            frames,
            segment_frames,
            index,
            cache: None,
        })
    }

    /// Decodes segment `n` into the cache.
    fn load_segment(&mut self, n: usize) -> io::Result<()> {
        if self.cache.as_ref().is_some_and(|(i, _)| *i == n) {
            return Ok(());
        }
        let (offset, length) = self.index[n];
        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0u8; length as usize];
        self.file.read_exact(&mut bytes)?;
        let samples = decode_flac(bytes).map_err(|e| bad(&e))?;
        let want = (self
            .segment_frames
            .min(self.frames - n as u64 * self.segment_frames)
            * self.channels as u64) as usize;
        if samples.len() < want {
            return Err(bad(
                "a segment of the compressed sound is shorter than it should be",
            ));
        }
        self.cache = Some((n, samples));
        Ok(())
    }

    fn read_samples(&mut self, start_frame: u64, out: &mut [i16]) -> io::Result<usize> {
        let mut written = 0;
        let mut frame = start_frame;
        while written < out.len() && frame < self.frames {
            let n = (frame / self.segment_frames) as usize;
            self.load_segment(n)?;
            let (_, samples) = self.cache.as_ref().expect("just loaded");
            let in_segment = ((frame - n as u64 * self.segment_frames) as usize) * self.channels;
            let segment_end = (self
                .segment_frames
                .min(self.frames - n as u64 * self.segment_frames)
                as usize)
                * self.channels;
            let take = (segment_end - in_segment).min(out.len() - written);
            out[written..written + take].copy_from_slice(&samples[in_segment..in_segment + take]);
            written += take;
            frame += (take / self.channels) as u64;
        }
        Ok(written)
    }
}

/// Decodes one FLAC stream to 16-bit samples, interleaved.
fn decode_flac(bytes: Vec<u8>) -> Result<Vec<i16>, String> {
    let mut reader = claxon::FlacReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let info = reader.streaminfo();
    if info.bits_per_sample != 16 {
        return Err("a compressed segment is not 16-bit".into());
    }
    let mut out = Vec::with_capacity(info.samples.unwrap_or(0) as usize * info.channels as usize);
    for sample in reader.samples() {
        out.push(sample.map_err(|e| e.to_string())? as i16);
    }
    Ok(out)
}

/// Encodes interleaved 16-bit samples as one FLAC stream.
fn encode_flac(samples: &[i16], channels: usize) -> Result<Vec<u8>, String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let wide: Vec<i32> = samples.iter().map(|s| *s as i32).collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| format!("{e:?}"))?;
    let source =
        flacenc::source::MemSource::from_samples(&wide, channels, 16, ENGINE_RATE as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| format!("{e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| format!("{e:?}"))?;
    Ok(sink.as_slice().to_vec())
}

/// Compresses the stored WAV sound at `source` into a segmented FLAC file at `destination`, a segment at a time (a
/// sound of hours never has to fit in memory). The file is only complete once this returns `Ok`.
pub fn compress(source: &Path, destination: &Path) -> Result<(), String> {
    let mut wav = Stored::open(source).map_err(|e| e.to_string())?;
    if !matches!(wav, Stored::Wav { .. }) {
        return Err("this sound is already compressed".into());
    }
    let (channels, frames) = (wav.channels(), wav.frames());
    let segments = frames.div_ceil(SEGMENT_FRAMES) as usize;
    let file = File::create(destination).map_err(|e| e.to_string())?;
    let mut out = BufWriter::new(file);
    let run = |out: &mut BufWriter<File>, wav: &mut Stored| -> Result<(), String> {
        let mut header = [0u8; HEADER_BYTES as usize];
        header[0..4].copy_from_slice(MAGIC);
        header[4] = VERSION;
        header[5] = channels as u8;
        header[8..16].copy_from_slice(&frames.to_le_bytes());
        header[16..20].copy_from_slice(&(SEGMENT_FRAMES as u32).to_le_bytes());
        header[20..24].copy_from_slice(&(segments as u32).to_le_bytes());
        out.write_all(&header).map_err(|e| e.to_string())?;
        // room for the table, filled in at the end
        out.write_all(&vec![0u8; segments * ENTRY_BYTES as usize])
            .map_err(|e| e.to_string())?;
        let mut table = Vec::with_capacity(segments * ENTRY_BYTES as usize);
        let mut offset = HEADER_BYTES + segments as u64 * ENTRY_BYTES;
        let mut samples = vec![0i16; SEGMENT_FRAMES as usize * channels];
        for n in 0..segments as u64 {
            let got = wav
                .read_samples(n * SEGMENT_FRAMES, &mut samples)
                .map_err(|e| e.to_string())?;
            let packed = encode_flac(&samples[..got], channels)?;
            out.write_all(&packed).map_err(|e| e.to_string())?;
            table.extend_from_slice(&offset.to_le_bytes());
            table.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            offset += packed.len() as u64;
        }
        out.flush().map_err(|e| e.to_string())?;
        let file = out.get_mut();
        file.seek(SeekFrom::Start(HEADER_BYTES))
            .map_err(|e| e.to_string())?;
        file.write_all(&table).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    };
    let result = run(&mut out, &mut wav);
    if result.is_err() {
        drop(out);
        let _ = std::fs::remove_file(destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundboard::wav::WavWriter;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("omu-stored-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Something that is not trivially compressible: a tone with a slow wobble and some noise.
    fn sample(i: u64, ch: u64) -> i16 {
        let t = i as f32 / 48_000.0;
        let tone = (t * 440.0 * std::f32::consts::TAU).sin() * 9000.0;
        let wobble = (t * 3.0 * std::f32::consts::TAU).sin() * 2000.0;
        let noise = ((i.wrapping_mul(2_654_435_761) >> 7) % 600) as f32 - 300.0;
        (tone + wobble + noise + ch as f32 * 500.0) as i16
    }

    fn write_wav(path: &Path, channels: usize, frames: u64) -> Vec<i16> {
        let mut all = Vec::new();
        let mut w = WavWriter::create(path, channels).unwrap();
        for f in 0..frames {
            for c in 0..channels as u64 {
                all.push(sample(f, c));
            }
        }
        w.write(&all).unwrap();
        w.finish().unwrap();
        all
    }

    #[test]
    fn a_compressed_sound_reads_back_exactly_from_any_position_and_takes_less_room() {
        let dir = temp("round");
        let wav = dir.join("a.wav");
        let packed = dir.join("a.omuf");
        // a little more than two segments (12.5 s), stereo
        let frames = SEGMENT_FRAMES * 5 / 2 + 123;
        let all = write_wav(&wav, 2, frames);
        compress(&wav, &packed).unwrap();
        assert!(
            std::fs::metadata(&packed).unwrap().len() < std::fs::metadata(&wav).unwrap().len(),
            "smaller on disk"
        );

        let mut s = Stored::open(&packed).unwrap();
        assert_eq!((s.channels(), s.frames()), (2, frames));
        // everything, in odd-sized reads that cross the segment seams
        let mut back = Vec::new();
        let mut at = 0u64;
        let mut buf = vec![0i16; 16_384];
        loop {
            let n = s.read_samples(at, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            back.extend_from_slice(&buf[..n]);
            at += (n / 2) as u64;
        }
        assert_eq!(back.len(), all.len());
        assert!(back == all, "lossless: every sample is what was stored");
        // a jump into the middle of the last segment, then past the end
        let mut part = vec![0i16; 1000];
        let from = SEGMENT_FRAMES * 2 + 50;
        assert_eq!(s.read_samples(from, &mut part).unwrap(), 1000);
        assert_eq!(part, &all[(from * 2) as usize..(from * 2) as usize + 1000]);
        assert_eq!(s.read_samples(frames, &mut part).unwrap(), 0);
        assert_eq!(
            s.read_samples(frames - 10, &mut part).unwrap(),
            20,
            "only what is left"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wav_opens_through_the_same_reader_and_mono_works() {
        let dir = temp("mono");
        let wav = dir.join("m.wav");
        let all = write_wav(&wav, 1, 30_000);
        let mut s = Stored::open(&wav).unwrap();
        assert_eq!((s.channels(), s.frames()), (1, 30_000));
        let mut back = vec![0i16; 30_000];
        assert_eq!(s.read_samples(0, &mut back).unwrap(), 30_000);
        assert_eq!(back, all);
        let packed = dir.join("m.omuf");
        compress(&wav, &packed).unwrap();
        let mut c = Stored::open(&packed).unwrap();
        let mut again = vec![0i16; 30_000];
        assert_eq!(c.read_samples(0, &mut again).unwrap(), 30_000);
        assert_eq!(again, all);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_not_a_compressed_sound_is_refused_and_a_failed_compression_leaves_nothing() {
        let dir = temp("bad");
        let packed = dir.join("x.omuf");
        let wav = dir.join("x.wav");
        write_wav(&wav, 1, 1000);
        compress(&wav, &packed).unwrap();
        // compressing what is already compressed is refused, and the target is not left half-made
        let twice = dir.join("y.omuf");
        assert!(compress(&packed, &twice).is_err());
        assert!(!twice.exists());
        // a truncated header
        let cut = dir.join("cut.omuf");
        std::fs::write(&cut, &std::fs::read(&packed).unwrap()[..10]).unwrap();
        assert!(Stored::open(&cut).is_err());
        // a segment that was cut off is an error when read, not a crash or silence
        let bytes = std::fs::read(&packed).unwrap();
        let torn = dir.join("torn.omuf");
        std::fs::write(&torn, &bytes[..bytes.len() - 20]).unwrap();
        let mut s = Stored::open(&torn).unwrap();
        let mut out = vec![0i16; 100];
        assert!(s.read_samples(0, &mut out).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
