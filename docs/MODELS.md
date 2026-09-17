# OpenAI transcription models

The product uses stable OpenAI model aliases so users receive the current compatible model behind each alias without an application update.

| Product choice | API model | Displayed estimate | Purpose |
| --- | --- | --- | --- |
| Economy | `gpt-4o-mini-transcribe` | about $0.003/minute | Lowest-cost everyday dictation |
| Maximum accuracy | `gpt-transcribe` | about $0.0045/minute | High-accuracy file transcription |

The local Costs panel stores the duration, model, documented rate and calculated USD cost for each successful transcription. It keeps all-time totals separately from the latest 500 detail rows. On the first launch with this feature, the app imports the bounded local transcription history once and labels those rows as estimates from history. Transcript text, audio and patient data are never copied into the usage ledger.

In Settings, AIDOO Control offers two voice modes. **Economy AI** uses the turn-based pipeline described below. It sends only completed speech turns, so silence is not transcribed or billed as audio input. The tradeoff is a short pause between a command and the reply.

The optional **GPT Live 1** mode uses `gpt-live-1` for a full-duplex WebRTC conversation with smoother interruption and simultaneous listening and speaking. Its base rate is **$0.05/minute** for the full active session, billed per second; backend model and tool usage are separate. The OpenAI Platform bill remains authoritative. See the official [GPT Live 1 model documentation](https://developers.openai.com/api/docs/models/gpt-live-1).

In Economy AI, a local voice activity detector keeps a short pre-roll, ends a turn after a natural pause and sends only that bounded WAV turn to `gpt-transcribe` at $0.0045/minute. Silence while the assistant waits is not uploaded or added to the transcription estimate.

The transcript is sent through the Responses API to `gpt-5.6-luna` with the same AIDOO function tools. The ledger records the reported uncached input, cached input, cache-write and output token counts at $0.20, $0.02, $0.25 and $1.20 per million tokens. Requests above 272,000 input tokens use the documented long-context multipliers. Simple patient-search and next-patient commands are routed locally to the appropriate function tool before the first model call, while the model still receives the verified result and produces the spoken confirmation.

Short responses use `gpt-4o-mini-tts` with the `marin` voice. The Speech endpoint returns audio without per-request usage data, so the interface marks its local TTS amount as an estimate based on the generated audio duration at approximately $0.015/minute. OpenAI Platform billing remains authoritative. The rates and APIs were checked on 17 September 2026 against the official [GPT-Transcribe](https://developers.openai.com/api/docs/models/gpt-transcribe), [GPT-5.6 Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna), [GPT-4o Mini TTS](https://developers.openai.com/api/docs/models/gpt-4o-mini-tts), [Create speech](https://developers.openai.com/api/reference/cli/resources/audio/subresources/speech/methods/create) and [Responses](https://developers.openai.com/api/reference/cli/resources/responses/methods/create) documentation.

The model IDs and price copy were checked on 15 September 2026 against the official [GPT-4o Mini Transcribe](https://developers.openai.com/api/docs/models/gpt-4o-mini-transcribe), [GPT-Transcribe](https://developers.openai.com/api/docs/models/gpt-transcribe) and [API pricing](https://developers.openai.com/api/docs/pricing) pages. Pricing is deliberately labelled approximate in the interface. Recheck these pages before every release because OpenAI can change availability and pricing independently from this application.

The app sends ISO-639-1 `language` only when the user selects an explicit language. In Automatic mode it omits the field and lets the model detect the language. OpenAI documents that an explicit language can improve accuracy and latency.

The app uploads the completed recording as FLAC with an extension-bearing filename and `audio/flac` content type. The official [Create transcription API reference](https://developers.openai.com/api/reference/python/resources/audio/subresources/transcriptions/methods/create) lists FLAC as a supported input for both selected models and documents the singular ISO-639-1 `language` parameter. Keeping FLAC reduces transfer size while preserving lossless local audio.

OpenAI's [File transcription guide](https://developers.openai.com/api/docs/guides/speech-to-text) limits files to 25 MB. The app applies the conservative decimal boundary of 25,000,000 bytes before upload, keeps an oversized recording locally and disables Retry because sending the unchanged file again cannot succeed.
