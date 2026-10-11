//! The "?" sound through waveOut: each play opens its own output, so sounds overlap like in the original.

use crate::wav::{self, Pcm};
use windows::Win32::Media::Audio::{
    CALLBACK_NULL, HWAVEOUT, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE, waveOutClose, waveOutOpen,
    waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader, waveOutWrite,
};
use windows::core::PSTR;

/// More overlapping sounds than this are skipped (the "?" still shows), as in the original.
const MAX_VOICES: usize = 8;

struct Voice {
    out: HWAVEOUT,
    /// Boxed: waveOut holds on to the header's address until the sound is done.
    hdr: Box<WAVEHDR>,
}

pub struct Sound {
    pcm: Pcm,
    voices: Vec<Voice>,
    /// Said once that there is no output, rather than on every ping.
    warned: bool,
}

impl Sound {
    pub fn new() -> Option<Self> {
        let mut pcm = wav::parse(include_bytes!("../../../assets/sounds/MIA.wav"))?;
        wav::scale(&mut pcm, wav::VOLUME);
        Some(Self { pcm, voices: Vec::new(), warned: false })
    }

    pub fn play(&mut self) {
        self.reap();
        if self.voices.len() >= MAX_VOICES {
            return;
        }
        let p = &self.pcm;
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: p.channels,
            nSamplesPerSec: p.rate,
            nAvgBytesPerSec: p.rate * p.block_align as u32,
            nBlockAlign: p.block_align,
            wBitsPerSample: p.bits,
            cbSize: 0,
        };
        let mut out = HWAVEOUT::default();
        unsafe {
            if waveOutOpen(Some(&mut out), WAVE_MAPPER, &fmt, None, None, CALLBACK_NULL) != 0 {
                if !std::mem::replace(&mut self.warned, true) {
                    log::warn!("no sound output");
                }
                return;
            }
            let mut hdr = Box::new(WAVEHDR {
                // waveOut only reads it.
                lpData: PSTR(p.data.as_ptr() as *mut u8),
                dwBufferLength: p.data.len() as u32,
                ..Default::default()
            });
            let size = std::mem::size_of::<WAVEHDR>() as u32;
            if waveOutPrepareHeader(out, &mut *hdr, size) != 0 || waveOutWrite(out, &mut *hdr, size) != 0 {
                let _ = waveOutUnprepareHeader(out, &mut *hdr, size);
                let _ = waveOutClose(out);
                return;
            }
            self.voices.push(Voice { out, hdr });
        }
    }

    /// Closes the outputs that have finished. True while any still plays.
    pub fn reap(&mut self) -> bool {
        self.voices.retain_mut(|v| {
            if v.hdr.dwFlags & WHDR_DONE == 0 {
                return true;
            }
            unsafe {
                let _ = waveOutUnprepareHeader(v.out, &mut *v.hdr, std::mem::size_of::<WAVEHDR>() as u32);
                let _ = waveOutClose(v.out);
            }
            false
        });
        !self.voices.is_empty()
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        for v in &mut self.voices {
            unsafe {
                let _ = waveOutReset(v.out);
                let _ = waveOutUnprepareHeader(v.out, &mut *v.hdr, std::mem::size_of::<WAVEHDR>() as u32);
                let _ = waveOutClose(v.out);
            }
        }
    }
}
