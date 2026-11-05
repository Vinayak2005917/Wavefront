use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{nemo, transcript::ServerEvent};

const NEMOTRON_DIR: &str = "nemotron-3.5-asr-streaming-0.6b-q8_0";
const NEMOTRON_REPO: &str = "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b/resolve/ea30d66debe3740a08b573244286791d423d6b3e";
const NEMOTRON_FILE: &str = "nemotron-3.5-asr-streaming-0.6b.q8_0.gguf";
const NEMOTRON_SHA256: &str = "3fc991d3badad7277c11030a7519832cddaf2057aafed6d4b25147e953a070b1";

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

pub struct AsrEngine {
    recognizer: nemo::Recognizer,
}

impl AsrEngine {
    pub fn load(app: &AppHandle, settings: &RecognizerSettings) -> Result<Self, String> {
        overlay::publish(
            app,
            "asr_checking",
            "Preparing Nemotron 3.5 ASR",
            "Checking for the local NVIDIA CPU runtime",
        );

        let runtime = nemo::Runtime::load()?;
        let model = ensure_nemotron_model(app, &settings.model_directory)?;
        overlay::publish(
            app,
            "asr_loading",
            "Loading Nemotron 3.5 ASR",
            "Initializing the CPU streaming recognizer",
        );
        let recognizer = runtime.create_recognizer(&model.join(NEMOTRON_FILE))?;
        Ok(Self { recognizer })
    }

    pub fn ready_state(&self) -> (&'static str, String) {
        (
            "Nemotron 3.5 ASR ready",
            "Local streaming recognition is running on the CPU".into(),
        )
    }

    pub fn model_name(&self) -> &'static str {
        "Nemotron 3.5"
    }

    pub fn create_stream(&self) -> Result<AsrStream, String> {
        Ok(AsrStream {
            stream: self.recognizer.create_stream()?,
            events: VecDeque::new(),
            committed: Vec::new(),
            last_partial: String::new(),
            segment: 0,
            input_sample_rate: None,
            ended: false,
        })
    }
}

pub enum ClientEvent {
    Server(ServerEvent),
    Closed,
}

pub struct AsrStream {
    stream: nemo::Stream,
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
        let updates = self.stream.push(samples, sample_rate)?;
        self.process_nemotron_updates(updates);
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if self.ended {
            return Ok(());
        }
        let updates = self.stream.finish()?;
        self.process_nemotron_updates(updates);
        if !self.last_partial.is_empty() {
            let text = std::mem::take(&mut self.last_partial);
            self.commit(text);
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
