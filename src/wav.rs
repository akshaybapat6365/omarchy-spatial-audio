//! Pure-std WAV reader & writer supporting IEEE 754 32-bit float (tag 3) and 16-bit PCM (tag 1).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct WavData {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples normalized to [-1.0, 1.0] (or raw float32).
    pub samples: Vec<f32>,
}

/// Write a standard IEEE 754 32-bit float WAV file (PipeWire convolver native format).
pub fn write_wav_f32<P: AsRef<Path>>(
    path: P,
    sample_rate: u32,
    channels: u16,
    interleaved: &[f32],
) -> io::Result<()> {
    let mut f = File::create(path)?;
    let bytes_per_sample: u16 = 4;
    let block_align: u16 = channels * bytes_per_sample;
    let byte_rate: u32 = sample_rate * (block_align as u32);
    let data_bytes: u32 = (interleaved.len() * 4) as u32;
    // RIFF header (12) + fmt chunk (24) + fact chunk (12) + data header (8) + data
    let riff_size: u32 = 4 + 24 + 12 + 8 + data_bytes;
    let num_frames: u32 = (interleaved.len() / (channels as usize)) as u32;

    f.write_all(b"RIFF")?;
    f.write_all(&riff_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;

    // fmt chunk (16 bytes payload, format tag 3 = WAVE_FORMAT_IEEE_FLOAT)
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;

    // fact chunk (required by RIFF WAVE spec for non-PCM float WAV)
    f.write_all(b"fact")?;
    f.write_all(&4u32.to_le_bytes())?;
    f.write_all(&num_frames.to_le_bytes())?;

    // data chunk
    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    for &s in interleaved {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

/// Read an IEEE 754 float32 or 16-bit PCM WAV file into interleaved `f32` samples.
pub fn read_wav<P: AsRef<Path>>(path: P) -> io::Result<WavData> {
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;

    if buf.len() < 12 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Not a RIFF/WAVE file"));
    }

    let mut pos = 12usize;
    let mut audio_fmt = 0u16;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut data_slice: Option<&[u8]> = None;

    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let sz = u32::from_le_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]) as usize;
        pos += 8;
        if pos + sz > buf.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Truncated WAV chunk"));
        }
        if id == b"fmt " {
            if sz < 16 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Short fmt chunk"));
            }
            audio_fmt = u16::from_le_bytes([buf[pos], buf[pos + 1]]);
            channels = u16::from_le_bytes([buf[pos + 2], buf[pos + 3]]);
            sample_rate = u32::from_le_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]);
            bits_per_sample = u16::from_le_bytes([buf[pos + 14], buf[pos + 15]]);
        } else if id == b"data" {
            data_slice = Some(&buf[pos..pos + sz]);
        }
        pos += sz + (sz & 1);
    }

    let raw = data_slice.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing data chunk"))?;
    let samples = match (audio_fmt, bits_per_sample) {
        (3, 32) => raw
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (1, 16) => raw
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        (1, 24) => raw
            .chunks_exact(3)
            .map(|c| {
                let mut v = (c[0] as i32) | ((c[1] as i32) << 8) | ((c[2] as i32) << 16);
                if v & 0x80_0000 != 0 {
                    v |= !0xFF_FFFF;
                }
                v as f32 / 8_388_608.0
            })
            .collect(),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Unsupported WAV format tag={audio_fmt} bps={bits_per_sample}"),
            ));
        }
    };

    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}
