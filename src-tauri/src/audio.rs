use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};

const FRAMES_PER_SECOND: usize = 50; // 20 ms frames at the capture device's native rate.
const WAVEFORM_POINTS: usize = 144;
const WAVEFORM_SAMPLE_RATE: usize = 72;
const WAVEFORM_EMIT_INTERVAL: Duration = Duration::from_millis(20);

pub struct AudioSession {
    capture: Option<Capture>,
    rate: usize,
    frame_samples: usize,
    frame_pending: Vec<f32>,
    frames: VecDeque<Vec<f32>>,
    waveform: VecDeque<f32>,
    waveform_phase: usize,
    waveform_sum: f32,
    last_waveform_publish: Instant,
}

struct Capture {
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    overruns: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    started: Instant,
    rate: usize,
}

impl Capture {
    fn open() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No default microphone is available")?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate() as usize;
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();
        let config = config.config();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let overruns = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let callback_samples = samples.clone();
        let callback_total = total.clone();
        let callback_overruns = overruns.clone();
        let max_samples = rate * 2;
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::I32 => build_stream::<i32>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::I24 => build_stream::<cpal::I24>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::U32 => build_stream::<u32>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::F64 => build_stream::<f64>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::U8 => build_stream::<u8>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            SampleFormat::I8 => build_stream::<i8>(
                &device,
                &config,
                channels,
                callback_samples,
                callback_total,
                callback_overruns,
                max_samples,
            ),
            format => return Err(format!("Unsupported microphone sample format: {format:?}")),
        }
        .map_err(|e| format!("Could not open microphone input: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("Could not start microphone input: {e}"))?;
        Ok(Self {
            stream,
            samples,
            overruns,
            total,
            started: Instant::now(),
            rate,
        })
    }
    fn take(&self) -> Vec<f32> {
        self.samples
            .lock()
            .map(|mut samples| std::mem::take(&mut *samples))
            .unwrap_or_default()
    }
    fn take_overruns(&self) -> usize {
        self.overruns.swap(0, Ordering::Relaxed)
    }
    fn finish(self, tail: Duration) -> (Vec<f32>, usize) {
        let want = ((self.started.elapsed() + tail).as_secs_f64() * self.rate as f64) as usize;
        let deadline = Instant::now() + tail + Duration::from_secs(1);
        while self.total.load(Ordering::Relaxed) < want && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let overruns = self.overruns.swap(0, Ordering::Relaxed);
        drop(self.stream);
        let samples = self
            .samples
            .lock()
            .map(|mut samples| std::mem::take(&mut *samples))
            .unwrap_or_default();
        (samples, overruns)
    }
}

fn build_stream<T: SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    samples: Arc<Mutex<Vec<f32>>>,
    total: Arc<AtomicUsize>,
    overruns: Arc<AtomicUsize>,
    max_samples: usize,
) -> Result<Stream, String>
where
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config.clone(),
            move |input: &[T], _| {
                let frames = input.len() / channels.max(1);
                total.fetch_add(frames, Ordering::Relaxed);
                if let Ok(mut buffer) = samples.lock() {
                    if channels == 1 {
                        buffer.extend(input.iter().map(|sample| f32::from_sample(*sample)));
                    } else {
                        for frame in input.chunks_exact(channels) {
                            buffer.push(
                                frame
                                    .iter()
                                    .map(|sample| f32::from_sample(*sample))
                                    .sum::<f32>()
                                    / channels as f32,
                            );
                        }
                    }
                    if buffer.len() > max_samples {
                        let drop_count = buffer.len() - max_samples;
                        buffer.drain(..drop_count);
                        overruns.fetch_add(drop_count, Ordering::Relaxed);
                    }
                }
            },
            move |error| eprintln!("[wavefront] microphone stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}
pub struct AudioUpdate {
    pub waveform: Option<Vec<f32>>,
    pub frames: Vec<Vec<f32>>,
    pub overrun_samples: usize,
}

impl AudioSession {
    pub fn open() -> Result<Self, String> {
        let capture = Capture::open()?;
        let rate = capture.rate;
        let frame_samples = ((rate as f64 / FRAMES_PER_SECOND as f64).round() as usize).max(1);
        Ok(Self {
            capture: Some(capture),
            rate,
            frame_samples,
            frame_pending: Vec::new(),
            frames: VecDeque::new(),
            waveform: VecDeque::with_capacity(WAVEFORM_POINTS),
            waveform_phase: 0,
            waveform_sum: 0.0,
            last_waveform_publish: Instant::now(),
        })
    }
    pub fn poll(&mut self) -> AudioUpdate {
        let Some(capture) = self.capture.as_ref() else {
            return self.consume_samples(&[]);
        };
        let overruns = capture.take_overruns();
        let samples = capture.take();
        let mut update = self.consume_samples(&samples);
        update.overrun_samples = overruns;
        update
    }
    pub fn sample_rate(&self) -> usize {
        self.rate
    }
    pub fn take_frames(&mut self) -> Vec<Vec<f32>> {
        self.frames.drain(..).collect()
    }
    pub fn stop(&mut self) -> AudioUpdate {
        let (tail, overruns) = self
            .capture
            .take()
            .map(|c| c.finish(Duration::from_millis(100)))
            .unwrap_or_default();
        let mut update = self.consume_samples(&tail);
        update.overrun_samples = overruns;
        if !self.frame_pending.is_empty() {
            self.frames
                .push_back(std::mem::take(&mut self.frame_pending));
        }
        update.frames = self.take_frames();
        update.waveform = Some(vec![0.0; WAVEFORM_POINTS]);
        update
    }
    fn consume_samples(&mut self, samples: &[f32]) -> AudioUpdate {
        self.update_waveform(samples);
        for &sample in samples {
            self.push_output(sample);
        }
        AudioUpdate {
            waveform: if self.last_waveform_publish.elapsed() >= WAVEFORM_EMIT_INTERVAL {
                self.last_waveform_publish = Instant::now();
                Some(self.waveform.iter().copied().collect())
            } else {
                None
            },
            frames: self.take_frames(),
            overrun_samples: 0,
        }
    }
    fn push_output(&mut self, sample: f32) {
        self.frame_pending.push(if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        });
        if self.frame_pending.len() >= self.frame_samples {
            self.frames
                .push_back(std::mem::take(&mut self.frame_pending));
        }
    }
    fn update_waveform(&mut self, samples: &[f32]) {
        let stride = (self.rate / WAVEFORM_SAMPLE_RATE).max(1);
        for sample in samples {
            let x = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.waveform_sum += x * x;
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
