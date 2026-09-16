# OpenAI transcription models

The product uses stable OpenAI model aliases so users receive the current compatible model behind each alias without an application update.

| Product choice | API model | Displayed estimate | Purpose |
| --- | --- | --- | --- |
| Economy | `gpt-4o-mini-transcribe` | about $0.003/minute | Lowest-cost everyday dictation |
| Maximum accuracy | `gpt-transcribe` | about $0.0045/minute | High-accuracy file transcription |

The local Costs panel stores the duration, model, documented rate and calculated USD cost for each successful transcription. It keeps all-time totals separately from the latest 500 detail rows. On the first launch with this feature, the app imports the bounded local transcription history once and labels those rows as estimates from history. Transcript text, audio and patient data are never copied into the usage ledger.

GPT-Live 1 voice sessions are measured from the successful Live connection until the app releases the session. The base voice session cost is calculated at $0.05/minute with millisecond precision; OpenAI bills the service per second without rounding to a whole minute. Backend Responses models and tool calls are charged separately by OpenAI, so the panel identifies that extra usage as excluded instead of presenting an incomplete value as the full invoice. The rate and billing behavior were checked on 16 September 2026 against the official [GPT-Live 1 model page](https://developers.openai.com/api/docs/models/gpt-live-1).

The model IDs and price copy were checked on 15 September 2026 against the official [GPT-4o Mini Transcribe](https://developers.openai.com/api/docs/models/gpt-4o-mini-transcribe), [GPT-Transcribe](https://developers.openai.com/api/docs/models/gpt-transcribe) and [API pricing](https://developers.openai.com/api/docs/pricing) pages. Pricing is deliberately labelled approximate in the interface. Recheck these pages before every release because OpenAI can change availability and pricing independently from this application.

The app sends ISO-639-1 `language` only when the user selects an explicit language. In Automatic mode it omits the field and lets the model detect the language. OpenAI documents that an explicit language can improve accuracy and latency.

The app uploads the completed recording as FLAC with an extension-bearing filename and `audio/flac` content type. The official [Create transcription API reference](https://developers.openai.com/api/reference/python/resources/audio/subresources/transcriptions/methods/create) lists FLAC as a supported input for both selected models and documents the singular ISO-639-1 `language` parameter. Keeping FLAC reduces transfer size while preserving lossless local audio.

OpenAI's [File transcription guide](https://developers.openai.com/api/docs/guides/speech-to-text) limits files to 25 MB. The app applies the conservative decimal boundary of 25,000,000 bytes before upload, keeps an oversized recording locally and disables Retry because sending the unchanged file again cannot succeed.
