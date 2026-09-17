const SPEECH_PCM_SAMPLE_RATE: u32 = 24_000;
const SPEECH_PCM_BYTES_PER_SAMPLE: u16 = 2;
const INVALID_SPEECH_FILE: &str = "OpenAI върна невалиден гласов файл.";

pub(crate) fn pcm_to_wav(pcm: &[u8]) -> Result<Vec<u8>, String> {
    if pcm.is_empty()
        || !pcm
            .len()
            .is_multiple_of(usize::from(SPEECH_PCM_BYTES_PER_SAMPLE))
    {
        return Err(INVALID_SPEECH_FILE.into());
    }
    let data_size = u32::try_from(pcm.len())
        .map_err(|_| "OpenAI върна прекалено дълъг гласов файл.".to_string())?;
    let riff_size = data_size
        .checked_add(36)
        .ok_or_else(|| "OpenAI върна прекалено дълъг гласов файл.".to_string())?;
    let byte_rate = SPEECH_PCM_SAMPLE_RATE * u32::from(SPEECH_PCM_BYTES_PER_SAMPLE);
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&SPEECH_PCM_SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&SPEECH_PCM_BYTES_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

pub(crate) fn wav_duration_seconds(audio: &[u8]) -> Result<f64, String> {
    if audio.len() < 12 || &audio[0..4] != b"RIFF" || &audio[8..12] != b"WAVE" {
        return Err(INVALID_SPEECH_FILE.into());
    }

    let mut offset = 12_usize;
    let mut bytes_per_second = None;
    let mut data_bytes = None;
    while offset.saturating_add(8) <= audio.len() {
        let chunk_id = &audio[offset..offset + 4];
        let declared_size = u32::from_le_bytes(
            audio[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| INVALID_SPEECH_FILE.to_string())?,
        );
        let payload_start = offset + 8;
        let available = audio.len().saturating_sub(payload_start);
        let actual_size = if declared_size == u32::MAX {
            available
        } else {
            usize::try_from(declared_size)
                .unwrap_or(usize::MAX)
                .min(available)
        };

        if chunk_id == b"fmt " {
            if actual_size < 16 {
                return Err(INVALID_SPEECH_FILE.into());
            }
            let rate = u32::from_le_bytes(
                audio[payload_start + 8..payload_start + 12]
                    .try_into()
                    .map_err(|_| INVALID_SPEECH_FILE.to_string())?,
            );
            if rate == 0 {
                return Err(INVALID_SPEECH_FILE.into());
            }
            bytes_per_second = Some(rate);
        } else if chunk_id == b"data" {
            data_bytes = Some(actual_size);
            break;
        }

        if declared_size == u32::MAX
            || usize::try_from(declared_size).unwrap_or(usize::MAX) > available
        {
            break;
        }
        offset = payload_start
            .saturating_add(actual_size)
            .saturating_add(actual_size % 2);
    }

    data_bytes
        .zip(bytes_per_second)
        .map(|(bytes, rate)| bytes as f64 / f64::from(rate))
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| INVALID_SPEECH_FILE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_duration_from_streaming_wav_with_unknown_chunk_sizes() {
        let mut wav = vec![0_u8; 44 + 48_000];
        write_test_header(&mut wav, u32::MAX, u32::MAX, 24_000);

        assert_eq!(wav_duration_seconds(&wav).unwrap(), 1.0);
    }

    #[test]
    fn wraps_openai_pcm_as_a_standard_wav() {
        let pcm = vec![0_u8; 48_000];
        let wav = pcm_to_wav(&pcm).unwrap();

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(wav_duration_seconds(&wav).unwrap(), 1.0);
    }

    #[test]
    fn rejects_truncated_pcm_sample() {
        assert!(pcm_to_wav(&[0_u8]).is_err());
    }

    #[test]
    fn rejects_wav_without_audio_data() {
        let mut wav = vec![0_u8; 36];
        write_test_header(&mut wav, 28, 0, 16_000);
        assert!(wav_duration_seconds(&wav).is_err());
    }

    fn write_test_header(wav: &mut [u8], riff_size: u32, data_size: u32, sample_rate: u32) {
        wav[0..4].copy_from_slice(b"RIFF");
        wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
        wav[8..12].copy_from_slice(b"WAVE");
        wav[12..16].copy_from_slice(b"fmt ");
        wav[16..20].copy_from_slice(&16_u32.to_le_bytes());
        wav[20..22].copy_from_slice(&1_u16.to_le_bytes());
        wav[22..24].copy_from_slice(&1_u16.to_le_bytes());
        wav[24..28].copy_from_slice(&sample_rate.to_le_bytes());
        wav[28..32].copy_from_slice(&(sample_rate * 2).to_le_bytes());
        wav[32..34].copy_from_slice(&2_u16.to_le_bytes());
        wav[34..36].copy_from_slice(&16_u16.to_le_bytes());
        if wav.len() >= 44 {
            wav[36..40].copy_from_slice(b"data");
            wav[40..44].copy_from_slice(&data_size.to_le_bytes());
        }
    }
}
