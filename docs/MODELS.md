# OpenAI transcription models

The product uses stable OpenAI model aliases so users receive the current compatible model behind each alias without an application update.

| Product choice | API model | Displayed estimate | Purpose |
| --- | --- | --- | --- |
| Economy | `gpt-4o-mini-transcribe` | about $0.003/minute | Lowest-cost everyday dictation |
| Maximum accuracy | `gpt-transcribe` | about $0.0045/minute | High-accuracy file transcription |

The model IDs and price copy were checked on 13 September 2026 against the official [GPT-4o Mini Transcribe](https://developers.openai.com/api/docs/models/gpt-4o-mini-transcribe) and [GPT-Transcribe](https://developers.openai.com/api/docs/models/gpt-transcribe) pages. Pricing is deliberately labelled approximate in the interface. Recheck both pages before every release because OpenAI can change availability and pricing independently from this application.

The app sends ISO-639-1 `language` only when the user selects an explicit language. In Automatic mode it omits the field and lets the model detect the language. OpenAI documents that an explicit language can improve accuracy and latency.
