use std::collections::VecDeque;
use std::time::{Duration, Instant};

use phonon::mic::{Capture, Vad};

const FRAME_MS: usize = 20;
const WINDOW_SECS: usize = 4;
const WAVEFORM_POINTS: usize = 144;
const WAVEFORM_SAMPLE_RATE: usize = 72;
const WAVEFORM_EMIT_INTERVAL: Duration = Duration::from_millis(20);

pub struct AudioSession {
    capture: Option<Capture>,
    rate: usize,
    frame_size: usize,
    pending: Vec<f32>,
    ring: VecDeque<f32>,
    total_samples: u64,
    vad: Vad,
    has_speech: bool,
    waveform: VecDeque<f32>,
    waveform_phase: usize,
    waveform_sum: f32,
    last_waveform_publish: Instant,
}

pub struct AudioUpdate {
    pub waveform: Option<Vec<f32>>,
}

impl AudioSession {
    pub fn open() -> Result<Self, String> {
        let capture = Capture::open(None).map_err(|error| error.to_string())?;
        let rate = capture.rate;
        Ok(Self {
            capture: Some(capture),
            rate,
            frame_size: rate * FRAME_MS / 1000,
            pending: Vec::new(),
            ring: VecDeque::with_capacity(rate * WINDOW_SECS),
            total_samples: 0,
            vad: Vad::new(rate, None),
            has_speech: false,
            waveform: VecDeque::with_capacity(WAVEFORM_POINTS),
            waveform_phase: 0,
            waveform_sum: 0.0,
            last_waveform_publish: Instant::now(),
        })
    }

    pub fn poll(&mut self) -> AudioUpdate {
        let samples = self.capture.as_ref().map(Capture::take).unwrap_or_default();
        self.consume_samples(&samples)
    }

    pub fn sample_rate(&self) -> usize {
        self.rate
    }

    pub fn has_speech(&self) -> bool {
        self.has_speech
    }

    pub fn window(&self) -> Vec<f32> {
        self.ring.iter().copied().collect()
    }

    pub fn window_len(&self) -> usize {
        self.ring.len()
    }

    pub fn window_start_seconds(&self) -> f32 {
        self.total_samples.saturating_sub(self.ring.len() as u64) as f32 / self.rate as f32
    }

    pub fn stop(&mut self) -> AudioUpdate {
        let tail = self
            .capture
            .take()
            .map(|capture| capture.finish(Duration::from_millis(90)))
            .unwrap_or_default();
        let mut update = self.consume_samples(&tail);
        self.flush_pending();
        update.waveform = Some(vec![0.0; WAVEFORM_POINTS]);
        update
    }

    fn consume_samples(&mut self, samples: &[f32]) -> AudioUpdate {
        self.update_waveform(samples);
        self.pending.extend_from_slice(samples);
        let max_samples = self.rate * WINDOW_SECS;
        let mut consumed = 0;
        while self.frame_size > 0 && self.pending.len().saturating_sub(consumed) >= self.frame_size
        {
            let frame = &self.pending[consumed..consumed + self.frame_size];
            self.has_speech |= self.vad.frame(frame) > 0;
            self.ring.extend(frame.iter().copied());
            self.total_samples += frame.len() as u64;
            while self.ring.len() > max_samples {
                self.ring.pop_front();
            }
            consumed += self.frame_size;
        }
        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        AudioUpdate {
            waveform: if self.last_waveform_publish.elapsed() >= WAVEFORM_EMIT_INTERVAL {
                self.last_waveform_publish = Instant::now();
                Some(self.waveform.iter().copied().collect())
            } else {
                None
            },
        }
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let tail: Vec<f32> = self.pending.drain(..).collect();
        self.total_samples += tail.len() as u64;
        self.ring.extend(tail);
        let max_samples = self.rate * WINDOW_SECS;
        while self.ring.len() > max_samples {
            self.ring.pop_front();
        }
    }

    fn update_waveform(&mut self, samples: &[f32]) {
        let stride = (self.rate / WAVEFORM_SAMPLE_RATE).max(1);
        for sample in samples {
            self.waveform_sum += sample * sample;
            self.waveform_phase += 1;
            if self.waveform_phase >= stride {
                self.waveform.push_back(
                    (self.waveform_sum / self.waveform_phase as f32)
                        .sqrt()
                        .clamp(0.0, 1.0),
                );
                while self.waveform.len() > WAVEFORM_POINTS {
                    self.waveform.pop_front();
                }
                self.waveform_phase = 0;
                self.waveform_sum = 0.0;
            }
        }
    }
}
