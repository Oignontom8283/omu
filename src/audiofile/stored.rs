//! Stored sounds in either shape they can have, behind one reader.
//!
//! * **WAV** (see [`crate::wav`]): the plain shape every imported sound starts in. The samples are the bytes of the
//!   file, so any position is a position in the file.
//! * **FLAC** (`.flac`): the same samples compressed without loss, for long sounds. It is an ordinary FLAC file that any
//!   player opens, with a seek table that has one point every [`SEGMENT_FRAMES`] frames (the length of the segments the
//!   reader works in), so that any segment can be read without reading what is before it. Nothing in it is lossy: what
//!   comes back is exactly the samples that went in.
//! * **Segmented FLAC** (`.omuf`): the shape compressed sounds had at first (a small header and independent FLAC streams).
//!   It is only read, and [`repack`] turns it into a FLAC file.
//!
//! ```text
//! legacy: "OMUF"  version(1)  channels(1)  reserved(2)  frames(8)  segment_frames(4)  segments(4)   = 24 bytes
//!         per segment: offset(8) length(4), then the FLAC streams (little-endian numbers)
//! ```

use crate::wav::{self, WavInfo};
use crate::{ENGINE_RATE, MAX_CHANNELS, SEGMENT_SECONDS};
use std::fs::File;
use std::io::{self, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

const LEGACY_MAGIC: &[u8; 4] = b"OMUF";
const FLAC_MARKER: &[u8; 4] = b"fLaC";
const LEGACY_HEADER_BYTES: u64 = 24;
const LEGACY_ENTRY_BYTES: u64 = 12;
/// Metadata blocks of a FLAC file: the kind of the one that holds the stream's facts, and of the seek table.
const STREAM_INFO: u8 = 0;
const SEEK_TABLE: u8 = 3;
/// A point of the seek table: sample number (8), byte offset from the first frame (8), samples in the frame (2).
const SEEK_POINT_BYTES: usize = 18;

/// Frames in a segment of a compressed sound (the segment the reader works in).
pub const SEGMENT_FRAMES: u64 = SEGMENT_SECONDS * ENGINE_RATE as u64;
/// Frames in one FLAC block: a segment is a whole number of them, so that every segment starts on a block.
const BLOCK_FRAMES: usize = 4000;
const _: () = assert!(SEGMENT_FRAMES.is_multiple_of(BLOCK_FRAMES as u64));

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
    /// What comes before the frames of a segment so that it can be decoded alone: the header of the file (a legacy file
    /// has a header in each segment instead).
    prefix: Vec<u8>,
    legacy: bool,
    /// The segment decoded last: reading goes forward, so one is enough.
    cache: Option<(usize, Vec<i16>)>,
}

impl Stored {
    /// Opens a stored sound of either shape.
    pub fn open(path: &Path) -> io::Result<Stored> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic == LEGACY_MAGIC {
            Ok(Stored::Flac(Box::new(FlacStored::read_legacy(file)?)))
        } else if &magic == FLAC_MARKER {
            Ok(Stored::Flac(Box::new(FlacStored::read_flac(file)?)))
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

    /// Whether it is in the first compressed shape (`.omuf`), which is only read.
    pub fn is_legacy(&self) -> bool {
        matches!(self, Stored::Flac(f) if f.legacy)
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
    fn read_legacy(mut file: File) -> io::Result<FlacStored> {
        let mut rest = [0u8; (LEGACY_HEADER_BYTES - 4) as usize];
        file.read_exact(&mut rest)?;
        if rest[0] != 1 {
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
        // the table must fit in the file: a header cannot make the program ask for more memory than the file is big
        if (segments as u64).saturating_mul(LEGACY_ENTRY_BYTES) > file.metadata()?.len() {
            return Err(bad("the compressed sound's header is not valid"));
        }
        let mut table = vec![0u8; segments * LEGACY_ENTRY_BYTES as usize];
        file.read_exact(&mut table)?;
        let index = table
            .as_chunks::<{ LEGACY_ENTRY_BYTES as usize }>()
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
            prefix: vec![],
            legacy: true,
            cache: None,
        })
    }

    /// Reads the metadata of a FLAC file (the marker is already read): the facts of the stream and the seek table.
    fn read_flac(mut file: File) -> io::Result<FlacStored> {
        let length = file.metadata()?.len();
        let mut header = FLAC_MARKER.to_vec();
        let (mut info, mut table) = (None, None);
        loop {
            let mut block = [0u8; 4];
            file.read_exact(&mut block)?;
            let size = u32::from_be_bytes([0, block[1], block[2], block[3]]) as usize;
            if size > 16 << 20 {
                return Err(bad("a block of the file's header is too large"));
            }
            let mut body = vec![0u8; size];
            file.read_exact(&mut body)?;
            header.extend_from_slice(&block);
            header.extend_from_slice(&body);
            match block[0] & 0x7f {
                STREAM_INFO => info = Some(body),
                SEEK_TABLE => table = Some(body),
                _ => {}
            }
            if block[0] & 0x80 != 0 {
                break;
            }
        }
        let info = info
            .filter(|i| i.len() == 34)
            .ok_or_else(|| bad("the file has no stream information"))?;
        // 20 bits of rate, 3 of channels, 5 of bits per sample, 36 of samples
        let facts = u64::from_be_bytes(info[10..18].try_into().expect("8 bytes"));
        let (rate, channels) = (facts >> 44, ((facts >> 41) & 7) as usize + 1);
        let (bits, frames) = (((facts >> 36) & 31) + 1, facts & ((1 << 36) - 1));
        if rate != ENGINE_RATE as u64 || bits != 16 || !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(bad("this FLAC file was not made by this program"));
        }
        let table = table.unwrap_or_default();
        let segments = frames.div_ceil(SEGMENT_FRAMES) as usize;
        if table.len() != segments * SEEK_POINT_BYTES {
            return Err(bad(
                "the seek table of the file is not the one this program makes",
            ));
        }
        let first = header.len() as u64;
        let mut starts = Vec::with_capacity(segments);
        for (n, point) in table.as_chunks::<SEEK_POINT_BYTES>().0.iter().enumerate() {
            let sample = u64::from_be_bytes(point[0..8].try_into().expect("8 bytes"));
            let offset = u64::from_be_bytes(point[8..16].try_into().expect("8 bytes"));
            if sample != n as u64 * SEGMENT_FRAMES || first + offset > length {
                return Err(bad("the seek table of the file is not valid"));
            }
            starts.push(first + offset);
        }
        let index = starts
            .iter()
            .enumerate()
            .map(|(n, &start)| {
                let end = starts.get(n + 1).copied().unwrap_or(length);
                (start, end.saturating_sub(start) as u32)
            })
            .collect();
        Ok(FlacStored {
            file,
            channels,
            frames,
            segment_frames: SEGMENT_FRAMES,
            index,
            prefix: header,
            legacy: false,
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
        let mut bytes = self.prefix.clone();
        let at = bytes.len();
        bytes.resize(at + length as usize, 0);
        self.file.read_exact(&mut bytes[at..])?;
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

/// Decodes FLAC frames (with the header before them) to 16-bit samples, interleaved.
fn decode_flac(bytes: Vec<u8>) -> Result<Vec<i16>, String> {
    let mut reader = claxon::FlacReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let info = reader.streaminfo();
    if info.bits_per_sample != 16 {
        return Err("a compressed segment is not 16-bit".into());
    }
    let mut out = Vec::with_capacity(SEGMENT_FRAMES as usize * info.channels as usize);
    for sample in reader.samples() {
        out.push(sample.map_err(|e| e.to_string())? as i16);
    }
    Ok(out)
}

/// The start of a FLAC file: the marker, the facts of the stream and (when there are segments) the seek table. It has the
/// same length whatever the numbers in it are, so that it can be written in place once the frames are known.
fn flac_header(
    info: &flacenc::component::StreamInfo,
    seek: &[(u64, u64)],
    frames: u64,
) -> Result<Vec<u8>, String> {
    use flacenc::component::{BitRepr, MetadataBlockData, Stream};
    let mut stream = Stream::with_stream_info(info.clone());
    if !seek.is_empty() {
        let mut table = Vec::with_capacity(seek.len() * SEEK_POINT_BYTES);
        for &(sample, offset) in seek {
            let in_it = (frames - sample).min(BLOCK_FRAMES as u64) as u16;
            table.extend_from_slice(&sample.to_be_bytes());
            table.extend_from_slice(&offset.to_be_bytes());
            table.extend_from_slice(&in_it.to_be_bytes());
        }
        stream.add_metadata_block(
            MetadataBlockData::new_unknown(SEEK_TABLE, &table).map_err(|e| format!("{e:?}"))?,
        );
    }
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| format!("{e:?}"))?;
    Ok(sink.as_slice().to_vec())
}

/// Compresses the stored WAV sound at `source` into a FLAC file at `destination`, a segment at a time (a sound of hours
/// never has to fit in memory). The file is only complete once this returns `Ok`.
pub fn compress(source: &Path, destination: &Path) -> Result<(), String> {
    let mut wav = Stored::open(source).map_err(|e| e.to_string())?;
    if !matches!(wav, Stored::Wav { .. }) {
        return Err("this sound is already compressed".into());
    }
    write_flac(&mut wav, destination)
}

/// Turns a sound in the first compressed shape (`.omuf`) into a FLAC file at `destination`.
pub fn repack(source: &Path, destination: &Path) -> Result<(), String> {
    let mut old = Stored::open(source).map_err(|e| e.to_string())?;
    if !old.is_legacy() {
        return Err("this sound is not in the old compressed shape".into());
    }
    write_flac(&mut old, destination)
}

fn write_flac(sound: &mut Stored, destination: &Path) -> Result<(), String> {
    use flacenc::component::{BitRepr, StreamInfo};
    use flacenc::error::Verify;
    use flacenc::source::{Fill, FrameBuf};
    let (channels, frames) = (sound.channels(), sound.frames());
    let segments = frames.div_ceil(SEGMENT_FRAMES) as usize;
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| format!("{e:?}"))?;
    let mut info =
        StreamInfo::new(ENGINE_RATE as usize, channels, 16).map_err(|e| format!("{e:?}"))?;
    let file = File::create(destination).map_err(|e| e.to_string())?;
    let mut out = BufWriter::new(file);
    let mut run = |out: &mut BufWriter<File>, sound: &mut Stored| -> Result<(), String> {
        // room for the header, written for good at the end
        let room = flac_header(&info, &vec![(0, 0); segments], frames)?.len();
        out.write_all(&vec![0u8; room]).map_err(|e| e.to_string())?;
        let mut seek = Vec::with_capacity(segments);
        let (mut written, mut number) = (0u64, 0usize);
        let mut samples = vec![0i16; SEGMENT_FRAMES as usize * channels];
        for n in 0..segments as u64 {
            let got = sound
                .read_samples(n * SEGMENT_FRAMES, &mut samples)
                .map_err(|e| e.to_string())?;
            seek.push((n * SEGMENT_FRAMES, written));
            for block in samples[..got].chunks(BLOCK_FRAMES * channels) {
                let wide: Vec<i32> = block.iter().map(|s| *s as i32).collect();
                let mut buffer = FrameBuf::with_size(channels, block.len() / channels)
                    .map_err(|e| format!("{e:?}"))?;
                buffer
                    .fill_interleaved(&wide)
                    .map_err(|e| format!("{e:?}"))?;
                let frame = flacenc::encode_fixed_size_frame(&config, &buffer, number, &info)
                    .map_err(|e| format!("{e:?}"))?;
                info.update_frame_info(&frame);
                let mut sink = flacenc::bitsink::ByteSink::new();
                frame.write(&mut sink).map_err(|e| format!("{e:?}"))?;
                out.write_all(sink.as_slice()).map_err(|e| e.to_string())?;
                written += sink.as_slice().len() as u64;
                number += 1;
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        let header = flac_header(&info, &seek, frames)?;
        if header.len() != room {
            return Err("the header of the compressed sound changed size".into());
        }
        let file = out.get_mut();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        file.write_all(&header).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    };
    let result = run(&mut out, sound);
    if result.is_err() {
        drop(out);
        let _ = std::fs::remove_file(destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::WavWriter;

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
        let packed = dir.join("a.flac");
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
    fn the_compressed_file_is_a_real_flac_file_any_decoder_reads() {
        let dir = temp("real");
        let wav = dir.join("r.wav");
        let packed = dir.join("r.flac");
        let frames = SEGMENT_FRAMES + 777;
        let all = write_wav(&wav, 2, frames);
        compress(&wav, &packed).unwrap();
        let mut reader = claxon::FlacReader::open(&packed).unwrap();
        assert_eq!(reader.streaminfo().sample_rate, ENGINE_RATE);
        assert_eq!(reader.streaminfo().samples, Some(frames));
        let back: Vec<i16> = reader.samples().map(|s| s.unwrap() as i16).collect();
        assert!(
            back == all,
            "read whole by an ordinary FLAC decoder, every sample is there"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The first compressed shape, as it was written.
    fn write_legacy(source: &Path, destination: &Path) {
        use flacenc::component::BitRepr;
        use flacenc::error::Verify;
        let mut wav = Stored::open(source).unwrap();
        let (channels, frames) = (wav.channels(), wav.frames());
        let segments = frames.div_ceil(SEGMENT_FRAMES) as usize;
        let mut head = vec![0u8; 24];
        head[0..4].copy_from_slice(LEGACY_MAGIC);
        head[4] = 1;
        head[5] = channels as u8;
        head[8..16].copy_from_slice(&frames.to_le_bytes());
        head[16..20].copy_from_slice(&(SEGMENT_FRAMES as u32).to_le_bytes());
        head[20..24].copy_from_slice(&(segments as u32).to_le_bytes());
        let (mut table, mut body) = (vec![], vec![]);
        let mut offset = 24 + segments as u64 * 12;
        let mut samples = vec![0i16; SEGMENT_FRAMES as usize * channels];
        for n in 0..segments as u64 {
            let got = wav.read_samples(n * SEGMENT_FRAMES, &mut samples).unwrap();
            let wide: Vec<i32> = samples[..got].iter().map(|s| *s as i32).collect();
            let config = flacenc::config::Encoder::default().into_verified().unwrap();
            let source = flacenc::source::MemSource::from_samples(&wide, channels, 16, 48_000);
            let stream =
                flacenc::encode_with_fixed_block_size(&config, source, config.block_size).unwrap();
            let mut sink = flacenc::bitsink::ByteSink::new();
            stream.write(&mut sink).unwrap();
            table.extend_from_slice(&offset.to_le_bytes());
            table.extend_from_slice(&(sink.as_slice().len() as u32).to_le_bytes());
            offset += sink.as_slice().len() as u64;
            body.extend_from_slice(sink.as_slice());
        }
        std::fs::write(destination, [head, table, body].concat()).unwrap();
    }

    #[test]
    fn an_old_header_that_claims_more_segments_than_the_file_holds_is_refused_without_asking_for_the_memory(
    ) {
        let dir = temp("hostile");
        let path = dir.join("h.omuf");
        let segments = u32::MAX;
        let frames = segments as u64 * SEGMENT_FRAMES;
        let mut head = vec![0u8; 24];
        head[0..4].copy_from_slice(LEGACY_MAGIC);
        head[4] = 1;
        head[5] = 2;
        head[8..16].copy_from_slice(&frames.to_le_bytes());
        head[16..20].copy_from_slice(&(SEGMENT_FRAMES as u32).to_le_bytes());
        head[20..24].copy_from_slice(&segments.to_le_bytes());
        std::fs::write(&path, head).unwrap();
        assert!(Stored::open(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sound_in_the_old_compressed_shape_still_reads_and_is_repacked_as_flac() {
        let dir = temp("legacy");
        let wav = dir.join("o.wav");
        let old = dir.join("o.omuf");
        let all = write_wav(&wav, 2, SEGMENT_FRAMES + 321);
        write_legacy(&wav, &old);
        let mut s = Stored::open(&old).unwrap();
        assert!(s.is_legacy());
        let mut back = vec![0i16; all.len()];
        assert_eq!(s.read_samples(0, &mut back).unwrap(), all.len());
        assert!(back == all);
        let now = dir.join("o.flac");
        repack(&old, &now).unwrap();
        assert!(
            repack(&now, &dir.join("again.flac")).is_err(),
            "only the old shape is repacked"
        );
        let mut f = Stored::open(&now).unwrap();
        assert!(!f.is_legacy());
        let mut again = vec![0i16; all.len()];
        assert_eq!(f.read_samples(0, &mut again).unwrap(), all.len());
        assert!(again == all);
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
        let packed = dir.join("m.flac");
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
        let packed = dir.join("x.flac");
        let wav = dir.join("x.wav");
        write_wav(&wav, 1, 1000);
        compress(&wav, &packed).unwrap();
        // compressing what is already compressed is refused, and the target is not left half-made
        let twice = dir.join("y.flac");
        assert!(compress(&packed, &twice).is_err());
        assert!(!twice.exists());
        // a truncated header
        let cut = dir.join("cut.flac");
        std::fs::write(&cut, &std::fs::read(&packed).unwrap()[..10]).unwrap();
        assert!(Stored::open(&cut).is_err());
        // a segment that was cut off is an error when read, not a crash or silence
        let bytes = std::fs::read(&packed).unwrap();
        let torn = dir.join("torn.flac");
        std::fs::write(&torn, &bytes[..bytes.len() - 20]).unwrap();
        let mut s = Stored::open(&torn).unwrap();
        let mut out = vec![0i16; 100];
        assert!(s.read_samples(0, &mut out).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
