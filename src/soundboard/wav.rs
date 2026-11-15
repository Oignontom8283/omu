//! The format sounds are stored in: WAV, PCM, 16 bits, 48 kHz, mono or stereo.
//!
//! It is the simplest format to read back (the samples are the bytes of the file, there is nothing to decode) and a
//! position in the sound is a position in the file, so a long sound can be read by segments from wherever. Only this
//! exact shape is read: whatever people import is converted to it once (see `import`).

use super::{ENGINE_RATE, MAX_CHANNELS};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// What the header of a stored sound says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub channels: usize,
    /// Frames (one sample per channel) in the file.
    pub frames: u64,
    /// Where the samples start in the file.
    pub data_offset: u64,
}

impl WavInfo {
    /// Bytes of samples.
    pub fn bytes(&self) -> u64 {
        self.frames * self.channels as u64 * 2
    }
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// Reads the header of a stored sound.
pub fn read_info(file: &mut File) -> io::Result<WavInfo> {
    file.seek(SeekFrom::Start(0))?;
    let mut riff = [0u8; 12];
    file.read_exact(&mut riff)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let (mut channels, mut ok_format) = (0usize, false);
    loop {
        let mut head = [0u8; 8];
        file.read_exact(&mut head)
            .map_err(|_| bad("the WAV file has no sound data"))?;
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as u64;
        match &head[0..4] {
            b"fmt " => {
                let mut f = vec![0u8; size as usize];
                file.read_exact(&mut f)?;
                if f.len() < 16 {
                    return Err(bad("the WAV format chunk is too short"));
                }
                let tag = u16::from_le_bytes([f[0], f[1]]);
                channels = u16::from_le_bytes([f[2], f[3]]) as usize;
                let rate = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
                let bits = u16::from_le_bytes([f[14], f[15]]);
                ok_format = tag == 1
                    && bits == 16
                    && rate == ENGINE_RATE
                    && (1..=MAX_CHANNELS).contains(&channels);
                if !ok_format {
                    return Err(bad(
                        "the stored sound is not 16-bit PCM at 48 kHz, mono or stereo",
                    ));
                }
                if size % 2 == 1 {
                    file.seek(SeekFrom::Current(1))?;
                }
            }
            b"data" => {
                if !ok_format {
                    return Err(bad("the WAV data comes before its format"));
                }
                let offset = file.stream_position()?;
                let available = file.metadata()?.len().saturating_sub(offset);
                // a header that was never finished (a crash while importing) says 0 or too much: trust the file
                let bytes = if size == 0 || size > available {
                    available
                } else {
                    size
                };
                let frame_bytes = channels as u64 * 2;
                return Ok(WavInfo {
                    channels,
                    frames: bytes / frame_bytes,
                    data_offset: offset,
                });
            }
            _ => {
                file.seek(SeekFrom::Current((size + (size & 1)) as i64))?;
            }
        }
    }
}

/// Reads up to `out.len()` samples (interleaved) starting at `start_frame`; returns how many samples were read.
pub fn read_samples(
    file: &mut File,
    info: &WavInfo,
    start_frame: u64,
    out: &mut [i16],
) -> io::Result<usize> {
    if start_frame >= info.frames {
        return Ok(0);
    }
    let wanted_frames = (out.len() / info.channels) as u64;
    let frames = wanted_frames.min(info.frames - start_frame) as usize;
    let samples = frames * info.channels;
    file.seek(SeekFrom::Start(
        info.data_offset + start_frame * info.channels as u64 * 2,
    ))?;
    let mut bytes = vec![0u8; samples * 2];
    file.read_exact(&mut bytes)?;
    for (o, b) in out.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *o = i16::from_le_bytes(*b);
    }
    Ok(samples)
}

/// Writes a stored sound a piece at a time (an imported video can be hours long: never all in memory), and fills in
/// the sizes of the header when finished.
pub struct WavWriter {
    file: io::BufWriter<File>,
    channels: usize,
    samples: u64,
}

/// The most a WAV file can hold (its sizes are 32 bits).
const MAX_DATA_BYTES: u64 = u32::MAX as u64 - 64;

impl WavWriter {
    pub fn create(path: &Path, channels: usize) -> io::Result<WavWriter> {
        assert!((1..=MAX_CHANNELS).contains(&channels));
        let mut file = io::BufWriter::new(File::create(path)?);
        file.write_all(&header(channels, 0))?;
        Ok(WavWriter {
            file,
            channels,
            samples: 0,
        })
    }

    pub fn write(&mut self, samples: &[i16]) -> io::Result<()> {
        if (self.samples + samples.len() as u64) * 2 > MAX_DATA_BYTES {
            return Err(bad("the sound is too long to be stored (about 5 hours)"));
        }
        for s in samples {
            self.file.write_all(&s.to_le_bytes())?;
        }
        self.samples += samples.len() as u64;
        Ok(())
    }

    /// Frames written so far.
    pub fn frames(&self) -> u64 {
        self.samples / self.channels as u64
    }

    /// Writes the real sizes into the header and flushes.
    pub fn finish(mut self) -> io::Result<()> {
        self.file.flush()?;
        let mut file = self.file.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(self.channels, self.samples * 2))?;
        file.sync_all()
    }
}

/// The 44 bytes that start a PCM WAV file with `data_bytes` of samples.
fn header(channels: usize, data_bytes: u64) -> [u8; 44] {
    let mut h = [0u8; 44];
    let data = data_bytes.min(u32::MAX as u64) as u32;
    let block = (channels * 2) as u16;
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&(channels as u16).to_le_bytes());
    h[24..28].copy_from_slice(&ENGINE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(ENGINE_RATE * block as u32).to_le_bytes());
    h[32..34].copy_from_slice(&block.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("omu-wav-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("sound.wav")
    }

    #[test]
    fn what_is_written_is_read_back_and_a_position_is_a_position_in_the_file() {
        let path = temp("round");
        let samples: Vec<i16> = (0..2000).map(|i| (i * 7 - 5000) as i16).collect();
        let mut w = WavWriter::create(&path, 2).unwrap();
        w.write(&samples[..1000]).unwrap();
        w.write(&samples[1000..]).unwrap();
        assert_eq!(w.frames(), 1000);
        w.finish().unwrap();

        let mut f = File::open(&path).unwrap();
        let info = read_info(&mut f).unwrap();
        assert_eq!(
            (info.channels, info.frames, info.data_offset),
            (2, 1000, 44)
        );
        assert_eq!(info.bytes(), 4000);
        let mut all = vec![0i16; 2000];
        assert_eq!(read_samples(&mut f, &info, 0, &mut all).unwrap(), 2000);
        assert_eq!(all, samples);
        // from the middle, and past the end
        let mut part = vec![0i16; 100];
        assert_eq!(read_samples(&mut f, &info, 500, &mut part).unwrap(), 100);
        assert_eq!(part, &samples[1000..1100]);
        let mut tail = vec![0i16; 100];
        assert_eq!(
            read_samples(&mut f, &info, 980, &mut tail).unwrap(),
            40,
            "only what is left"
        );
        assert_eq!(read_samples(&mut f, &info, 1000, &mut tail).unwrap(), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_header_that_was_never_finished_is_still_read() {
        // the program stopped while importing: the sizes in the header are still 0, the samples are there
        let path = temp("unfinished");
        let mut w = WavWriter::create(&path, 1).unwrap();
        w.write(&[1, 2, 3, 4, 5, 6]).unwrap();
        drop(w); // never finished
        let mut f = File::open(&path).unwrap();
        let info = read_info(&mut f).unwrap();
        assert_eq!(info.frames, 6, "the header said 0, the file holds 6");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn other_shapes_are_refused_with_a_reason() {
        let path = temp("other");
        // 44.1 kHz
        let mut h = header(2, 0);
        h[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        std::fs::write(&path, h).unwrap();
        let err = read_info(&mut File::open(&path).unwrap()).unwrap_err();
        assert!(err.to_string().contains("48 kHz"), "{err}");
        // not a WAV at all
        std::fs::write(&path, b"not a wav file at all, just text").unwrap();
        assert!(read_info(&mut File::open(&path).unwrap()).is_err());
        // extra chunks before the data are skipped
        let mut bytes = header(1, 4).to_vec();
        let data = bytes.split_off(36);
        bytes.extend_from_slice(b"LIST\x03\0\0\0abc\0"); // odd size, padded
        bytes.extend_from_slice(&data);
        bytes.extend_from_slice(&[9, 0, 8, 0]);
        std::fs::write(&path, &bytes).unwrap();
        let mut f = File::open(&path).unwrap();
        let info = read_info(&mut f).unwrap();
        assert_eq!(info.frames, 2);
        let mut s = [0i16; 2];
        read_samples(&mut f, &info, 0, &mut s).unwrap();
        assert_eq!(s, [9, 8]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
