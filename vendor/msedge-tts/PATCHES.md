# Local integration patches

Based on upstream `msedge-tts` 0.4.1 (repository recorded in Cargo.toml). The upstream MIT license is retained unchanged.

Local changes include Edge consumer protocol compatibility, audio-frame/header boundary validation, and Tokio streaming error handling. A transport close or EOF before `turn.end` must fail rather than report a completed speech turn. Regression tests cover truncated streams, malformed frames and normal metadata/audio/end sequences.

The application enables `tokio-runtime` with default features disabled. This is the tested integration; optional blocking, proxy, smol and documentation-example configurations are not represented as fully supported by the application's test run.
