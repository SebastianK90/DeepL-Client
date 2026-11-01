# Local security and dependency patch

Upstream: hs-CN/msedge-tts 0.4.1, commit 79d6d83c8e1ac6507b37f775055e7a4abbe78a82.
MIT license reproduced in LICENSE. This is a source patch, not a new upstream release.

Changes:
- Reject binary frames shorter than the two-byte length prefix or declared header.
- Regression tests exercise malformed headers and a valid audio boundary.
- The shared payload parser also rejects transport Close as a speech completion;
  only a protocol `turn.end` may complete a speech turn.
- The Tokio streaming receiver rejects transport Close/EOF before speech
  `turn.end`, preventing empty-read loops and falsely successful truncated audio.
  Four in-memory WebSocket regressions cover both premature termination paths
  and preserve metadata/audio/turn.end processing.
- Use the application's reqwest 0.12 without default features and the single ring
  TLS provider; align rustls-platform-verifier to 0.6.2. TLS verification is retained.
- The app uses only the Tokio backend; optional upstream backends remain source-only.

The framing check happens before conversion to SynthesizedResponse, which keeps
the downstream audio slice within bounds. Upstream protocol/token generation is
unchanged so Edge streaming and the existing male-voice recovery remain available.
