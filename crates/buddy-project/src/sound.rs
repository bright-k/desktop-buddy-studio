/// Check that an uploaded sound starts with a playable stream of the declared type.
pub fn validate_sound(extension: &str, bytes: &[u8]) -> Result<(), String> {
    let valid = match extension {
        "wav" => wav(bytes).map(|_| ()),
        "mp3" => mp3_payload(bytes).and_then(mp3_frame).map(|_| ()),
        _ => None,
    };
    valid.ok_or_else(|| "expected a PCM WAV or MPEG Layer III audio file".into())
}

struct PcmFrames {
    count: u64,
    rate: u32,
    channels: u16,
}

impl PcmFrames {
    fn decoded_bytes(&self, sample_rate: u32) -> u64 {
        self.count
            .saturating_mul(u64::from(sample_rate))
            .div_ceil(u64::from(self.rate))
            .saturating_mul(u64::from(self.channels) * 4)
    }
}

/// Only send audio to Web Audio when its decoded PCM fits the cache budget.
/// Streams with unknown lengths can still play from their original files.
pub fn sound_fits_decode_budget(
    extension: &str,
    bytes: &[u8],
    sample_rate: u32,
    budget: u32,
) -> bool {
    if !(8_000..=384_000).contains(&sample_rate) || budget == 0 {
        return false;
    }
    let budget = u64::from(budget);
    if bytes.len() as u64 > budget {
        return false;
    }
    if extension.eq_ignore_ascii_case("wav") {
        return wav(bytes)
            .is_some_and(|frames| frames.decoded_bytes(sample_rate.max(frames.rate)) <= budget);
    }
    if !extension.eq_ignore_ascii_case("mp3") {
        return false;
    }
    let Some(mut payload) = mp3_payload(bytes) else {
        return false;
    };
    let Some((_, first)) = mp3_frame(payload) else {
        return false;
    };
    let mut total = PcmFrames { count: 0, ..first };
    while !payload.is_empty() && !(payload.len() == 128 && payload.starts_with(b"TAG")) {
        let Some((length, frame)) = mp3_frame(payload) else {
            return false;
        };
        if frame.rate != total.rate || frame.channels != total.channels {
            return false;
        }
        total.count = total.count.saturating_add(frame.count);
        if total.decoded_bytes(sample_rate.max(total.rate)) > budget {
            return false;
        }
        payload = &payload[length..];
    }
    true
}

fn wav(bytes: &[u8]) -> Option<PcmFrames> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let end = 8usize.checked_add(u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize)?;
    let mut chunks = bytes.get(12..end)?;
    let mut format = None;
    let mut data = None;
    while !chunks.is_empty() {
        let size = u32::from_le_bytes(chunks.get(4..8)?.try_into().ok()?) as usize;
        let end = 8usize.checked_add(size)?;
        let body = chunks.get(8..end)?;
        match &chunks[..4] {
            b"fmt " => format = Some(body),
            b"data" => data = Some(body.len()),
            _ => {}
        }
        chunks = chunks.get(end.checked_add(size % 2)?..)?;
    }
    let format = format?;
    let tag = u16::from_le_bytes(format.get(..2)?.try_into().ok()?);
    let channels = u16::from_le_bytes(format.get(2..4)?.try_into().ok()?);
    let rate = u32::from_le_bytes(format.get(4..8)?.try_into().ok()?);
    let alignment = u16::from_le_bytes(format.get(12..14)?.try_into().ok()?);
    let bits = u16::from_le_bytes(format.get(14..16)?.try_into().ok()?);
    let data = data?;
    let sample_format = match tag {
        1 => matches!(bits, 8 | 16 | 24 | 32),
        3 => bits == 32,
        _ => false,
    };
    (sample_format
        && (1..=2).contains(&channels)
        && (8000..=192000).contains(&rate)
        && alignment == channels * (bits / 8)
        && data > 0
        && data.is_multiple_of(usize::from(alignment)))
    .then(|| PcmFrames {
        count: (data / usize::from(alignment)) as u64,
        rate,
        channels,
    })
}

fn mp3_payload(bytes: &[u8]) -> Option<&[u8]> {
    let mut start = 0;
    if bytes.starts_with(b"ID3") {
        let length = bytes.get(6..10)?;
        if length.iter().any(|&byte| byte >= 128) {
            return None;
        }
        start = 10
            + length
                .iter()
                .fold(0usize, |size, &byte| (size << 7) | usize::from(byte));
        if bytes[3] == 4 && bytes[5] & 0x10 != 0 {
            start += 10;
        }
    }
    bytes.get(start..)
}

fn mp3_frame(bytes: &[u8]) -> Option<(usize, PcmFrames)> {
    let header = bytes.get(..4)?;
    if header[0] != 0xff || header[1] & 0xe6 != 0xe2 {
        return None;
    }
    let version = (header[1] >> 3) & 3;
    let index = usize::from(header[2] >> 4);
    let rate_index = usize::from((header[2] >> 2) & 3);
    if version == 1 || !(1..=14).contains(&index) || rate_index == 3 {
        return None;
    }
    let rates = [44100u32, 48000, 32000];
    let mut rate = rates[rate_index];
    let (samples, bitrate) = if version == 3 {
        (
            144000,
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ][index],
        )
    } else {
        rate /= if version == 2 { 2 } else { 4 };
        (
            72000,
            [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][index],
        )
    };
    let length = (samples * bitrate / rate) as usize + usize::from((header[2] >> 1) & 1);
    bytes.get(..length)?;
    Some((
        length,
        PcmFrames {
            count: if version == 3 { 1152 } else { 576 },
            rate,
            channels: if header[3] >> 6 == 3 { 1 } else { 2 },
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_budget_counts_pcm_at_both_source_and_output_rates() {
        let mut wav = b"RIFF".to_vec();
        wav.extend(8036u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend([1, 0, 1, 0]); // PCM, mono
        wav.extend(8_000u32.to_le_bytes());
        wav.extend(8_000u32.to_le_bytes());
        wav.extend([1, 0, 8, 0]); // One byte per sample
        wav.extend(b"data");
        wav.extend(8_000u32.to_le_bytes());
        wav.resize(8_044, 128);
        assert!(validate_sound("wav", &wav).is_ok());
        assert!(sound_fits_decode_budget("WAV", &wav, 48_000, 192_000));
        assert!(!sound_fits_decode_budget("wav", &wav, 48_000, 191_999));
        wav[24..28].copy_from_slice(&48_000u32.to_le_bytes());
        wav[28..32].copy_from_slice(&48_000u32.to_le_bytes());
        assert!(sound_fits_decode_budget("wav", &wav, 8_000, 32_000));
        assert!(!sound_fits_decode_budget("wav", &wav, 8_000, 31_999));
    }

    #[test]
    fn mp3_budget_counts_all_frames_and_leaves_unmeasurable_streams_playable() {
        let mut frame = vec![0; 417]; // MPEG-1 Layer III, 128 kbps at 44.1 kHz
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        let mut mp3 = frame.repeat(2);
        mp3.extend(b"TAG");
        mp3.extend([0; 125]);
        assert!(validate_sound("mp3", &mp3).is_ok());
        assert!(sound_fits_decode_budget("mp3", &mp3, 44_100, 2 * 1152 * 8));
        assert!(!sound_fits_decode_budget(
            "mp3",
            &mp3,
            44_100,
            2 * 1152 * 8 - 1
        ));
        mp3.push(0);
        assert!(validate_sound("mp3", &mp3).is_ok());
        assert!(!sound_fits_decode_budget(
            "mp3",
            &mp3,
            44_100,
            8 * 1024 * 1024
        ));
    }
}
