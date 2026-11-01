# Wavefront patches

This is upstream `phonon-rs` 0.1.3 (Apache-2.0), copied from the published crate. Wavefront adds `Capture::level_handle()`, an atomic RMS value updated by the existing microphone callback, and caps the callback's pending sample buffer at two seconds by dropping its oldest samples if the consumer falls behind. The UI reads the RMS value on a small independent thread, so the waveform continues to respond while the CPU recognizer is processing a phrase.

Upstream: https://github.com/apiplant/phonon-rs
