/// Check that an uploaded sound starts with a playable stream of the declared type.
pub fn validate_sound(extension: &str, bytes: &[u8]) -> Result<(), String> {
    let valid = match extension {
        "wav" => wav(bytes),
        "mp3" => mp3(bytes),
        _ => None,
    };
    valid.ok_or_else(|| "expected a PCM WAV or MPEG Layer III audio file".into())
}

fn wav(bytes: &[u8]) -> Option<()> {
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
    .then_some(())
}

fn mp3(bytes: &[u8]) -> Option<()> {
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
    let header = bytes.get(start..start + 4)?;
    if header[0] != 0xff || header[1] & 0xe6 != 0xe2 {
        return None;
    }
    let version = (header[1] >> 3) & 3;
    let index = usize::from(header[2] >> 4);
    let rate_index = usize::from((header[2] >> 2) & 3);
    if version == 1 || !(1..=14).contains(&index) || rate_index == 3 {
        return None;
    }
    let rates = [44100, 48000, 32000];
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
    let length = samples * bitrate / rate + usize::from((header[2] >> 1) & 1);
    bytes.get(start..start + length).map(|_| ())
}
