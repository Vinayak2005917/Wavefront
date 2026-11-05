use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use tauri::{AppHandle, Manager};

use crate::{nemo, transcript::ServerEvent};

const NEMOTRON_DIR: &str = "nemotron-3.5-asr-streaming-0.6b-q8_0";
const NEMOTRON_REPO: &str = "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b/resolve/ea30d66debe3740a08b573244286791d423d6b3e";
const NEMOTRON_FILE: &str = "nemotron-3.5-asr-streaming-0.6b.q8_0.gguf";
const NEMOTRON_SHA256: &str = "3fc991d3badad7277c11030a7519832cddaf2057aafed6d4b25147e953a070b1";

const ZIPFORMER_DIR: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
const ZIPFORMER_REPO: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main";
const ZIPFORMER_FILES: [&str; 4] = [
    "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
    // Keep the existing FP32 decoder correction in the Zipformer fallback.
    "decoder-epoch-99-avg-1-chunk-16-left-128.onnx",
    "joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
    "tokens.txt",
];

#[derive(Clone, Debug)]
pub struct RecognizerSettings {
    pub model_directory: PathBuf,
}

impl RecognizerSettings {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let app_dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
        let value = fs::read_to_string(app_dir.join("model-directory.txt"))
            .or_else(|_| fs::read_to_string(app_dir.join("zipformer-model-directory.txt")))
            .or_else(|_| fs::read_to_string(app_dir.join("fermion-cache-dir.txt")))
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Ok(Self {
            model_directory: value
                .map(PathBuf::from)
                .unwrap_or_else(|| app_dir.join("models")),
        })
    }

    pub fn save(app: &AppHandle, directory: &Path) -> Result<(), String> {
        let app_dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
        fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
        fs::write(
            app_dir.join("model-directory.txt"),
            directory.to_string_lossy().as_bytes(),
        )
        .map_err(|error| format!("Could not save model directory: {error}"))
    }
}

pub fn normalize_model_directory(path: &str) -> Result<PathBuf, String> {
    let value = path.trim();
    if value.is_empty() {
        return Err("Choose a folder for the local speech models.".into());
    }
    let path = PathBuf::from(value);
    fs::create_dir_all(&path).map_err(|e| format!("Could not create model folder: {e}"))?;
    Ok(path)
}

enum RecognizerBackend {
    Nemotron(Arc<nemo::Recognizer>),
    Zipformer(Arc<OnlineRecognizer>),
}

pub struct AsrEngine {
    backend: RecognizerBackend,
    fallback_reason: Option<String>,
}

impl AsrEngine {
    pub fn load(app: &AppHandle, settings: &RecognizerSettings) -> Result<Self, String> {
        overlay::publish(
            app,
            "asr_checking",
            "Preparing Nemotron 3.5 ASR",
            "Checking for the local NVIDIA CPU runtime",
        );

        if std::env::var("WAVEFRONT_ASR_BACKEND")
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case("zipformer"))
        {
            return load_zipformer(app, settings).map(|recognizer| Self {
                backend: RecognizerBackend::Zipformer(Arc::new(recognizer)),
                fallback_reason: Some(
                    "Zipformer was selected with WAVEFRONT_ASR_BACKEND=zipformer.".into(),
                ),
            });
        }

        let nemotron = (|| {
            let runtime = nemo::Runtime::load()?;
            let model = ensure_nemotron_model(app, &settings.model_directory)?;
            overlay::publish(
                app,
                "asr_loading",
                "Loading Nemotron 3.5 ASR",
                "Initializing the CPU streaming recognizer",
            );
            runtime.create_recognizer(&model.join(NEMOTRON_FILE))
        })();

        match nemotron {
            Ok(recognizer) => Ok(Self {
                backend: RecognizerBackend::Nemotron(Arc::new(recognizer)),
                fallback_reason: None,
            }),
            Err(nemotron_error) => {
                eprintln!(
                    "[wavefront] Nemotron setup failed; loading Zipformer fallback: {nemotron_error}"
                );
                overlay::publish(
                    app,
                    "asr_loading",
                    "Loading Zipformer fallback",
                    &format!("Nemotron 3.5 is unavailable: {nemotron_error}"),
                );
                let recognizer = load_zipformer(app, settings).map_err(|zipformer_error| {
                    format!(
                        "Nemotron 3.5 is unavailable ({nemotron_error}); Zipformer fallback also failed ({zipformer_error})."
                    )
                })?;
                Ok(Self {
                    backend: RecognizerBackend::Zipformer(Arc::new(recognizer)),
                    fallback_reason: Some(nemotron_error),
                })
            }
        }
    }

    pub fn ready_state(&self) -> (&'static str, String) {
        match (&self.backend, &self.fallback_reason) {
            (RecognizerBackend::Nemotron(_), _) => (
                "Nemotron 3.5 ASR ready",
                "Local streaming recognition is running on the CPU".into(),
            ),
            (RecognizerBackend::Zipformer(_), Some(reason)) => (
                "Zipformer fallback ready",
                format!("Nemotron 3.5 is unavailable: {reason}"),
            ),
            (RecognizerBackend::Zipformer(_), None) => (
                "Zipformer ready",
                "The local streaming fallback is loaded".into(),
            ),
        }
    }

    pub fn model_name(&self) -> &'static str {
        match &self.backend {
            RecognizerBackend::Nemotron(_) => "Nemotron 3.5",
            RecognizerBackend::Zipformer(_) => "Zipformer fallback",
        }
    }

    pub fn create_stream(&self) -> Result<AsrStream, String> {
        let backend = match &self.backend {
            RecognizerBackend::Nemotron(recognizer) => {
                StreamBackend::Nemotron(recognizer.create_stream()?)
            }
            RecognizerBackend::Zipformer(recognizer) => StreamBackend::Zipformer {
                stream: recognizer.create_stream(),
                recognizer: recognizer.clone(),
            },
        };
        Ok(AsrStream {
            backend,
            events: VecDeque::new(),
            committed: Vec::new(),
            last_partial: String::new(),
            segment: 0,
            input_sample_rate: None,
            ended: false,
        })
    }
}

fn load_zipformer(
    app: &AppHandle,
    settings: &RecognizerSettings,
) -> Result<OnlineRecognizer, String> {
    overlay::publish(
        app,
        "asr_checking",
        "Preparing Zipformer fallback",
        "Checking local compatibility model files",
    );
    let model = ensure_zipformer_model(app, &settings.model_directory)?;
    overlay::publish(
        app,
        "asr_loading",
        "Loading Zipformer fallback",
        "Initializing the compatibility streaming recognizer",
    );
    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(
        model
            .join("encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx")
            .to_string_lossy()
            .into_owned(),
    );
    config.model_config.transducer.decoder = Some(
        model
            .join("decoder-epoch-99-avg-1-chunk-16-left-128.onnx")
            .to_string_lossy()
            .into_owned(),
    );
    config.model_config.transducer.joiner = Some(
        model
            .join("joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx")
            .to_string_lossy()
            .into_owned(),
    );
    config.model_config.tokens = Some(model.join("tokens.txt").to_string_lossy().into_owned());
    config.model_config.provider = Some("cpu".into());
    config.enable_endpoint = true;
    config.decoding_method = Some("greedy_search".into());
    OnlineRecognizer::create(&config)
        .ok_or_else(|| "Could not load the English Zipformer compatibility model.".to_owned())
}

pub enum ClientEvent {
    Server(ServerEvent),
    Closed,
}

enum StreamBackend {
    Nemotron(nemo::Stream),
    Zipformer {
        stream: OnlineStream,
        recognizer: Arc<OnlineRecognizer>,
    },
}

pub struct AsrStream {
    backend: StreamBackend,
    events: VecDeque<ClientEvent>,
    committed: Vec<String>,
    last_partial: String,
    segment: u64,
    input_sample_rate: Option<i32>,
    ended: bool,
}

impl AsrStream {
    pub fn send_audio(&mut self, samples: &[f32], sample_rate: usize) -> Result<(), String> {
        if self.ended {
            return Err("The recognition stream has already ended.".into());
        }
        if samples.is_empty() {
            return Ok(());
        }
        let sample_rate = i32::try_from(sample_rate)
            .map_err(|_| "The microphone sample rate is outside the supported range.".to_owned())?;
        if let Some(previous_rate) = self.input_sample_rate {
            if previous_rate != sample_rate {
                return Err("The microphone sample rate changed during dictation.".into());
            }
        } else {
            self.input_sample_rate = Some(sample_rate);
        }
        let nemotron_updates = match &mut self.backend {
            StreamBackend::Nemotron(stream) => Some(stream.push(samples, sample_rate)?),
            StreamBackend::Zipformer { stream, .. } => {
                stream.accept_waveform(sample_rate, samples);
                None
            }
        };
        if let Some(updates) = nemotron_updates {
            self.process_nemotron_updates(updates);
        } else {
            self.decode_zipformer_available();
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if self.ended {
            return Ok(());
        }
        let nemotron_updates = match &mut self.backend {
            StreamBackend::Nemotron(stream) => Some(stream.finish()?),
            StreamBackend::Zipformer { .. } => None,
        };
        if let Some(updates) = nemotron_updates {
            self.process_nemotron_updates(updates);
            if !self.last_partial.is_empty() {
                let text = std::mem::take(&mut self.last_partial);
                self.commit(text);
            }
        } else {
            self.finish_zipformer();
        }
        let full = self.committed.join(" ");
        self.events
            .push_back(ClientEvent::Server(ServerEvent::Done { text: full }));
        self.events.push_back(ClientEvent::Closed);
        self.ended = true;
        Ok(())
    }

    pub fn try_event(&mut self) -> Result<ClientEvent, mpsc::TryRecvError> {
        self.events.pop_front().ok_or(mpsc::TryRecvError::Empty)
    }

    fn decode_zipformer_available(&mut self) {
        let mut updates = Vec::new();
        if let StreamBackend::Zipformer { stream, recognizer } = &self.backend {
            while recognizer.is_ready(stream) {
                recognizer.decode(stream);
                let text = recognizer
                    .get_result(stream)
                    .map(|result| result.text.trim().to_owned())
                    .unwrap_or_default();
                let endpoint = recognizer.is_endpoint(stream);
                updates.push((text, endpoint));
                if endpoint {
                    recognizer.reset(stream);
                }
            }
        }
        for (text, endpoint) in updates {
            self.process_zipformer_text(text);
            if endpoint {
                self.finalize_partial();
            }
        }
    }

    fn finish_zipformer(&mut self) {
        let (updates, final_text) = {
            let mut updates = Vec::new();
            let mut final_text = String::new();
            if let StreamBackend::Zipformer { stream, recognizer } = &self.backend {
                let sample_rate = self.input_sample_rate.unwrap_or(16_000);
                let padding = vec![0.0f32; sample_rate as usize * 3 / 10];
                stream.accept_waveform(sample_rate, &padding);
                stream.input_finished();
                while recognizer.is_ready(stream) {
                    recognizer.decode(stream);
                    let text = recognizer
                        .get_result(stream)
                        .map(|result| result.text.trim().to_owned())
                        .unwrap_or_default();
                    let endpoint = recognizer.is_endpoint(stream);
                    updates.push((text, endpoint));
                    if endpoint {
                        recognizer.reset(stream);
                    }
                }
                final_text = recognizer
                    .get_result(stream)
                    .map(|result| result.text.trim().to_owned())
                    .unwrap_or_default();
            }
            (updates, final_text)
        };
        for (text, endpoint) in updates {
            self.process_zipformer_text(text);
            if endpoint {
                self.finalize_partial();
            }
        }
        if !final_text.is_empty() && final_text != self.last_partial {
            self.commit(final_text);
            self.last_partial.clear();
        } else {
            self.finalize_partial();
        }
    }

    fn process_zipformer_text(&mut self, text: String) {
        if !text.is_empty() && text != self.last_partial {
            self.last_partial = text.clone();
            self.events
                .push_back(ClientEvent::Server(ServerEvent::Partial { text }));
        }
    }

    fn process_nemotron_updates(&mut self, updates: Vec<nemo::ResultUpdate>) {
        for update in updates {
            let text = update.text.trim().to_owned();
            if update.is_final {
                if !text.is_empty() {
                    self.commit(text);
                } else {
                    self.finalize_partial();
                }
                self.last_partial.clear();
                self.segment = self.segment.wrapping_add(1);
            } else if text != self.last_partial {
                self.last_partial = text.clone();
                self.events
                    .push_back(ClientEvent::Server(ServerEvent::Partial { text }));
            }
        }
    }

    fn finalize_partial(&mut self) {
        if !self.last_partial.is_empty() {
            let text = std::mem::take(&mut self.last_partial);
            self.commit(text);
        }
        self.segment = self.segment.wrapping_add(1);
    }

    fn commit(&mut self, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.committed.push(text.clone());
        self.events
            .push_back(ClientEvent::Server(ServerEvent::Final {
                text,
                segment: Some(self.segment),
            }));
    }
}

use crate::overlay;

fn ensure_nemotron_model(app: &AppHandle, base: &Path) -> Result<PathBuf, String> {
    let directory = base.join(NEMOTRON_DIR);
    let model_path = directory.join(NEMOTRON_FILE);
    if model_path.is_file() && verify_nemotron_model(&model_path)? {
        return Ok(directory);
    }
    if model_path.exists() {
        fs::remove_file(&model_path)
            .map_err(|error| format!("Could not replace the invalid Nemotron model: {error}"))?;
    }

    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the Nemotron model folder: {error}"))?;
    overlay::publish(
        app,
        "asr_downloading",
        "Downloading Nemotron 3.5 Q8_0",
        "Downloading the 742 MB model from NVIDIA on Hugging Face",
    );
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(600))
        .build();
    download_file(
        app,
        &agent,
        &directory,
        NEMOTRON_FILE,
        &format!("{NEMOTRON_REPO}/{NEMOTRON_FILE}"),
        "Downloading Nemotron 3.5 Q8_0",
        1,
        1,
    )?;
    if !verify_nemotron_model(&model_path)? {
        let _ = fs::remove_file(&model_path);
        return Err(
            "The Nemotron download failed its SHA-256 check; choose Reload model to try again."
                .into(),
        );
    }
    Ok(directory)
}

fn verify_nemotron_model(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("Could not read the local Nemotron model: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not verify the Nemotron model: {error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual = format!("{:x}", hasher.finalize());
    Ok(actual == NEMOTRON_SHA256)
}

fn ensure_zipformer_model(app: &AppHandle, base: &Path) -> Result<PathBuf, String> {
    let model = base.join(ZIPFORMER_DIR);
    if ZIPFORMER_FILES
        .iter()
        .all(|file| model.join(file).is_file())
    {
        return Ok(model);
    }

    fs::create_dir_all(&model)
        .map_err(|error| format!("Could not create model folder: {error}"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build();
    let pending = ZIPFORMER_FILES
        .iter()
        .filter(|file| !model.join(file).is_file())
        .count();
    let mut current = 0;
    overlay::publish(
        app,
        "asr_downloading",
        "Downloading Zipformer fallback files",
        "Starting compatibility model download",
    );
    for file_name in ZIPFORMER_FILES {
        if model.join(file_name).is_file() {
            continue;
        }
        current += 1;
        download_file(
            app,
            &agent,
            &model,
            file_name,
            &format!("{ZIPFORMER_REPO}/{file_name}"),
            "Downloading Zipformer fallback files",
            current,
            pending,
        )?;
    }
    Ok(model)
}

fn download_file(
    app: &AppHandle,
    agent: &ureq::Agent,
    directory: &Path,
    file_name: &str,
    url: &str,
    title: &str,
    file_index: usize,
    file_count: usize,
) -> Result<(), String> {
    let destination = directory.join(file_name);
    let partial = directory.join(format!("{file_name}.part"));
    let previous_size = fs::metadata(&partial)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut request = agent.get(url);
    if previous_size > 0 {
        request = request.set("Range", &format!("bytes={previous_size}-"));
    }
    let response = request.call().map_err(|error| {
        format!("Could not download {file_name} from the model repository: {error}")
    })?;
    let resumed = response.status() == 206;
    let offset = if resumed { previous_size } else { 0 };
    let expected_size = response
        .header("Content-Length")
        .and_then(|length| length.parse::<u64>().ok())
        .map(|length| length + offset);
    let mut output = if resumed {
        OpenOptions::new()
            .append(true)
            .open(&partial)
            .map_err(|error| format!("Could not resume {file_name}: {error}"))?
    } else {
        File::create(&partial).map_err(|error| format!("Could not create {file_name}: {error}"))?
    };
    let mut reader = response.into_reader();
    let mut downloaded = offset;
    let mut last_publish = Instant::now() - Duration::from_secs(1);
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Download of {file_name} stopped: {error}"))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("Could not save {file_name}: {error}"))?;
        downloaded += count as u64;
        if last_publish.elapsed() >= Duration::from_millis(500) {
            let detail = match expected_size {
                Some(total) if total > 0 => format!(
                    "File {file_index} of {file_count} · {} / {} MB ({}%)",
                    downloaded / 1_000_000,
                    total / 1_000_000,
                    downloaded.saturating_mul(100) / total,
                ),
                _ => format!(
                    "File {file_index} of {file_count} · {} MB downloaded",
                    downloaded / 1_000_000,
                ),
            };
            overlay::publish(app, "asr_downloading", title, &detail);
            last_publish = Instant::now();
        }
    }
    if expected_size.is_some_and(|total| downloaded != total) {
        return Err(format!(
            "Download of {file_name} ended early ({downloaded} bytes received). Select Reload model to retry."
        ));
    }
    output
        .sync_all()
        .map_err(|error| format!("Could not finish saving {file_name}: {error}"))?;
    drop(output);
    fs::rename(&partial, &destination)
        .map_err(|error| format!("Could not finalize {file_name}: {error}"))?;
    Ok(())
}
