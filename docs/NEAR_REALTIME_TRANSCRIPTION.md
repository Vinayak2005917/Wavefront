# Wavefront Near-Real-Time Transcription Decisions (superseded)

This document records the original batch-transcription design and is kept as project history. Wavefront now uses an in-process sherpa-onnx English streaming Zipformer recognizer. See [Zipformer setup](ZIPFORMER_STREAMING.md) for the current architecture.

This document records the initial implementation decisions for exploring near-real-time transcription. Values are starting targets for trial and error, not guarantees.

## Architecture decisions

- **Update cadence:** Run a transcription update every 300 ms.
- **Audio window:** Transcribe a constant rolling 4-second window. Keep the implementation simple; do not grow the window with utterance length.
- **Inference constraint:** The current `phonon-rs` API does not expose incremental decoder state or encoder-state reuse. Each update therefore re-transcribes its rolling window. Capture must continue independently while inference runs.
- **Transcript state:** Keep the session transcript independent of the rolling audio buffer. Use absolute word timestamps to align each hypothesis with prior words; preserve the transcript prefix after its audio leaves the window. Keep separate committed and partial words, and retain the last three committed words as a rollback window.
- **Commit rule:** Commit a word after it remains unchanged for three consecutive updates. The rollback window must keep recently committed words revisable.
- **Field insertion:** Do not show transcript text in the flyout or make UI changes. Type words into the target field individually as transcription updates arrive. When a hypothesis changes, remove the changed suffix with word-level backspaces and type the replacement words individually. This assumes the user does not edit the inserted text or move the cursor during dictation.
- **Token comparison and output:** Split on whitespace, keeping punctuation, contractions, and hyphenated words attached. Normalize spacing for comparisons, but preserve the model's original text when typing. If punctuation or spacing changes, replace from the last matching complete-word boundary.
- **VAD behavior:** Keep the rolling window active across pauses; do not reset it at each VAD pause.

## Trial targets

Measure all three on the target laptop:

- **Word error rate:** target 10–15% or lower.
- **Update latency:** median at or below 300 ms; p95 at or below 500 ms.
- **CPU use:** at or below 50% sustained.

Prioritize transcription accuracy and latency over CPU use if the targets conflict. Record the target laptop and test conditions with each trial so results can be compared meaningfully.

The 300 ms cadence is best effort: only one inference runs at a time, so a slower inference delays the next update rather than building a queue of stale windows. The accuracy, latency, and CPU targets still need measurement on the target laptop.
