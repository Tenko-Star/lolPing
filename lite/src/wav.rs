//! The "?" sound: a 16-bit PCM WAV, read once and turned down to the original app's default volume.

/// The original app's default volume.
pub const VOLUME: f32 = 0.7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub block_align: u16,
    pub data: Vec<u8>,
}

/// The format and samples of a plain PCM WAV, or None for anything else.
pub fn parse(bytes: &[u8]) -> Option<Pcm> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().ok()?) as usize;
        let body = bytes.get(i + 8..(i + 8).checked_add(len)?)?;
        match id {
            b"fmt " if len >= 16 => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    let (f, d) = (fmt?, data?);
    let u16_at = |o: usize| u16::from_le_bytes([f[o], f[o + 1]]);
    if u16_at(0) != 1 || u16_at(14) != 16 {
        return None; // not 16-bit PCM
    }
    Some(Pcm {
        channels: u16_at(2),
        rate: u32::from_le_bytes(f[4..8].try_into().ok()?),
        bits: 16,
        block_align: u16_at(12),
        data: d.to_vec(),
    })
}

/// Scales every 16-bit sample.
pub fn scale(pcm: &mut Pcm, volume: f32) {
    for s in pcm.data.as_chunks_mut::<2>().0 {
        let v = i16::from_le_bytes([s[0], s[1]]) as f32 * volume;
        s.copy_from_slice(&(v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_bundled_sound() {
        let pcm = parse(include_bytes!("../../assets/sounds/MIA.wav")).unwrap();
        assert_eq!((pcm.channels, pcm.rate, pcm.bits, pcm.block_align), (2, 44_100, 16, 4));
        assert!(pcm.data.len() > 100_000);
        assert_eq!(pcm.data.len() % 4, 0);
    }

    #[test]
    fn scales_samples() {
        let mut pcm = Pcm { channels: 1, rate: 8000, bits: 16, block_align: 2, data: vec![] };
        for v in [1000i16, -1000, i16::MAX, i16::MIN] {
            pcm.data.extend_from_slice(&v.to_le_bytes());
        }
        scale(&mut pcm, 0.5);
        let out: Vec<i16> = pcm.data.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(out, vec![500, -500, 16384, -16384]);
    }

    #[test]
    fn refuses_what_it_cannot_play() {
        assert_eq!(parse(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(parse(b"not a wav at all"), None);
        let mut float = include_bytes!("../../assets/sounds/MIA.wav").to_vec();
        float[20] = 3; // fmt's format tag: IEEE float
        assert_eq!(parse(&float), None);
    }
}
