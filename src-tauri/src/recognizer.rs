use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use tauri::{AppHandle, Manager};

use crate::transcript::ServerEvent;

const MODEL_DIR: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
const MODEL_REPO: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main";
const MODEL_FILES: [&str; 4] = [
    "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
    "decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
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
        let value = fs::read_to_string(app_dir.join("zipformer-model-directory.txt"))
            .or_else(|_| fs::read_to_string(app_dir.join("fermion-cache-dir.txt")))
            .or_else(|_| fs::read_to_string(app_dir.join("model-directory.txt")))
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
            app_dir.join("zipformer-model-directory.txt"),
            directory.to_string_lossy().as_bytes(),
        )
        .map_err(|e| format!("Could not save model directory: {e}"))
    }
}

pub fn normalize_model_directory(path: &str) -> Result<PathBuf, String> {
    let value = path.trim();
    if value.is_empty() {
        return Err("Choose a folder for the local Zipformer model.".into());
    }
    let path = PathBuf::from(value);
    fs::create_dir_all(&path).map_err(|e| format!("Could not create model folder: {e}"))?;
    Ok(path)
}

pub struct AsrEngine {
    recognizer: Arc<OnlineRecognizer>,
}

impl AsrEngine {
    pub fn load(app: &AppHandle, settings: &RecognizerSettings) -> Result<Self, String> {
        overlay::publish(
            app,
            "asr_checking",
            "Preparing English Zipformer",
            "Checking local model files",
        );
        let model = ensure_model(app, &settings.model_directory)?;
        overlay::publish(
            app,
            "asr_loading",
            "Loading English Zipformer",
            "Initializing the streaming recognizer",
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
                .join("decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx")
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
        let recognizer = OnlineRecognizer::create(&config)
            .ok_or_else(|| "Could not load the English Zipformer model.".to_owned())?;
        Ok(Self {
            recognizer: Arc::new(recognizer),
        })
    }

    pub fn create_stream(&self) -> AsrStream {
        AsrStream {
            stream: self.recognizer.create_stream(),
            recognizer: self.recognizer.clone(),
            events: VecDeque::new(),
            committed: Vec::new(),
            last_partial: String::new(),
            segment: 0,
            ended: false,
        }
    }
}

pub enum ClientEvent {
    Server(ServerEvent),
    Closed,
}

pub struct AsrStream {
    stream: OnlineStream,
    recognizer: Arc<OnlineRecognizer>,
    events: VecDeque<ClientEvent>,
    committed: Vec<String>,
    last_partial: String,
    segment: u64,
    ended: bool,
}

impl AsrStream {
    pub fn send_audio(&mut self, samples: &[f32]) -> Result<(), String> {
        if self.ended {
            return Err("The recognition stream has already ended.".into());
        }
        if !samples.is_empty() {
            self.stream.accept_waveform(16_000, samples);
            self.decode_available();
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if self.ended {
            return Ok(());
        }
        let padding = vec![0.0f32; 4_800];
        self.stream.accept_waveform(16_000, &padding);
        self.stream.input_finished();
        self.decode_available();
        let text = self
            .recognizer
            .get_result(&self.stream)
            .map(|result| result.text)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !text.is_empty() && text != self.last_partial {
            self.commit(text);
        } else if !self.last_partial.is_empty() && self.committed.last() != Some(&self.last_partial)
        {
            self.commit(self.last_partial.clone());
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

    fn decode_available(&mut self) {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
            let text = self
                .recognizer
                .get_result(&self.stream)
                .map(|result| result.text.trim().to_owned())
                .unwrap_or_default();
            if !text.is_empty() && text != self.last_partial {
                self.last_partial = text.clone();
                self.events
                    .push_back(ClientEvent::Server(ServerEvent::Partial { text }));
            }
            if self.recognizer.is_endpoint(&self.stream) {
                if !self.last_partial.is_empty() {
                    self.commit(self.last_partial.clone());
                }
                self.recognizer.reset(&self.stream);
                self.last_partial.clear();
                self.segment = self.segment.wrapping_add(1);
            }
        }
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

fn ensure_model(app: &AppHandle, base: &Path) -> Result<PathBuf, String> {
    let model = base.join(MODEL_DIR);
    if MODEL_FILES.iter().all(|file| model.join(file).is_file()) {
        return Ok(model);
    }

    fs::create_dir_all(&model).map_err(|e| format!("Could not create model folder: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build();
    let pending = MODEL_FILES
        .iter()
        .filter(|file| !model.join(file).is_file())
        .count();
    let mut current = 0;
    overlay::publish(
        app,
        "asr_downloading",
        "Downloading English Zipformer files",
        "Starting model download",
    );
    for file_name in MODEL_FILES {
        if model.join(file_name).is_file() {
            continue;
        }
        current += 1;
        download_model_file(app, &agent, &model, file_name, current, pending)?;
    }
    Ok(model)
}

fn download_model_file(
    app: &AppHandle,
    agent: &ureq::Agent,
    directory: &Path,
    file_name: &str,
    file_index: usize,
    file_count: usize,
) -> Result<(), String> {
    let destination = directory.join(file_name);
    let partial = directory.join(format!("{file_name}.part"));
    let previous_size = fs::metadata(&partial)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let url = format!("{MODEL_REPO}/{file_name}");
    let mut request = agent.get(&url);
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
            overlay::publish(
                app,
                "asr_downloading",
                "Downloading English Zipformer files",
                &detail,
            );
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
