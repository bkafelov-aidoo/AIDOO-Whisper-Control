# Hey, AIDOO wake-word decision

## Decision

Use `livekit-wakeword` 0.1.3 behind AIDOO's own `WakeWordService` boundary. The detector runs entirely on device. The application keeps only a two-second in-memory PCM ring buffer, never writes that buffer to disk, and never sends pre-trigger audio over the network. The bundled `hey_aidoo.onnx` classifier proposes a candidate and `hey_aidoo_confirmation.onnx` confirms the phrase 250–750 ms later. The mel and embedding models are compiled into the Rust dependency.

This choice avoids an end-user or vendor access key and is Apache-2.0 licensed. The official Rust runtime supports both macOS and Windows. The dependency is pinned because its upstream status is beta; replacing it does not change the recording or transcription pipeline.

## Rejected alternatives

- Picovoice Porcupine has a mature small runtime and custom phrase models, but it requires a Picovoice AccessKey and commercial-license review. A key embedded in a desktop binary cannot be kept secret.
- sherpa-onnx supports local keyword spotting on macOS and Windows, but its Rust integration is third party and adds a larger native runtime and model set.
- rustpotter is small and Rust native, but its own project description says it is not intended as a production-grade tool.
- Apple Speech, Sound Analysis, and NSSpeechRecognizer would make the implementation Apple-specific and cannot be reused for the planned Windows product.

## Product behavior

Voice activation is opt-in and disabled by default. When enabled, macOS displays its normal microphone privacy indicator because the input device remains active. “Hey, AIDOO” plays a local acknowledgement sound and starts the existing protected recording flow. The wake phrase and acknowledgement precede the saved recording. The user can stop from the overlay. If automatic stop is enabled, 1.5 seconds of silence ends a voice-started recording. If no speech follows within five seconds, the recording is deleted without an OpenAI request.

The English phrase “Hey, AIDOO” is acoustically identical to “hey, I do”. No classifier can distinguish identical audio from intent alone. Evaluation therefore includes nearby phrases, while the exact homophone remains an inherent false-trigger case that must be covered in user-facing documentation.

## Release gates

The classifier is releasable only with recorded evidence of:

- recall of at least 90% on held-out positive speakers, with 95% as the target;
- no more than 0.125 false positives per hour (one per eight hours) on general negative audio;
- trigger-to-overlay latency below 1 second after the phrase ends;
- idle CPU near or below 1% on the oldest supported Apple Silicon test device;
- no persisted or transmitted audio before detection;
- recovery after sleep/wake and selected-microphone disconnect/reconnect.

Synthetic metrics are a model-development gate. A short real-speaker and real-room acceptance pass remains mandatory before publishing a signed build.

## Training provenance

The checked-in classifiers are trained with the adjacent `hey_aidoo.yaml` configuration and the LiveKit wake-word training repository at commit `95448a7559c453fcd87645bd67b247ffb45f85b0` (2 August 2026). The primary model uses phrase-at-end examples. The confirmation model uses separate 500 ms shifted positives and general-speech negatives. The training environment uses Python 3.11 and the project's locked dependencies. Release metadata records both model hashes, both thresholds and the continuous-stream evaluation so the shipped binary can be traced back to the evaluated pair.

After export and evaluation, `scripts/finalize-wakeword-model.py` copies both evaluated ONNX classifiers into the application resources and writes `hey_aidoo_model.json`. The command refuses incomplete training splits, mismatched model hashes, or an evaluation that was not produced by the two-stage temporal detector.

Synthetic speech comes from the `en_US-libritts_r-medium` Piper voice. Its model card lists the voice repository as MIT licensed and LibriTTS-R as the CC BY 4.0 training dataset. The bundled `MODEL-NOTICES.txt` records the source, license link, transformation and requested dataset citation. The application does not distribute the Piper model or source recordings.
