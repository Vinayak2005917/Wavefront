use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Instant;

use phonon::audio;
use phonon::engine::{DeviceArg, Engine};
use tauri::{AppHandle, Manager};

use crate::overlay;

const MODEL_ARCHIVE: &str = "phonon-2.bps.tar.zst";
const MODEL_URL: &str =
    "https://huggingface.co/FermionResearch/Phonon-2/resolve/main/phonon-2.bps.tar.zst";
const MODEL_MAX_BYTES: u64 = 512 * 1024 * 1024;

pub struct Recognizer {
    engine: Engine,
}

impl Recognizer {
    pub fn model_directory(app: &AppHandle) -> Result<PathBuf, String> {
        let app_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|error| error.to_string())?;
        let settings = app_dir.join("model-directory.txt");
        if let Ok(value) = fs::read_to_string(settings) {
            let path = PathBuf::from(value.trim());
            if !path.as_os_str().is_empty() {
                return Ok(path);
            }
        }
        Ok(app_dir.join("models").join("Phonon-2"))
    }

    pub fn save_model_directory(app: &AppHandle, directory: &PathBuf) -> Result<(), String> {
        let app_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|error| error.to_string())?;
        let previous = Self::model_directory(app)?;
        fs::create_dir_all(directory)
            .map_err(|error| format!("Could not create model folder: {error}"))?;
        let old_archive = previous.join(MODEL_ARCHIVE);
        let new_archive = directory.join(MODEL_ARCHIVE);
        if previous.as_path() != directory.as_path()
            && old_archive.is_file()
            && !new_archive.exists()
        {
            fs::copy(&old_archive, &new_archive).map_err(|error| {
                format!("Could not copy the existing model to this folder: {error}")
            })?;
        }
        fs::create_dir_all(&app_dir).map_err(|error| error.to_string())?;
        fs::write(
            app_dir.join("model-directory.txt"),
            directory.to_string_lossy().as_bytes(),
        )
        .map_err(|error| format!("Could not save model location: {error}"))
    }

    pub fn load(
        app: &AppHandle,
        directory: Option<PathBuf>,
        allow_download: bool,
    ) -> Result<Self, String> {
        let started = Instant::now();
        eprintln!("[wavefront] model load requested (allow_download={allow_download})");
        overlay::publish(
            app,
            "checking",
            "Preparing Phonon-2",
            "Checking the local model",
        );
        let model_path = directory
            .map(|directory| directory.join(MODEL_ARCHIVE))
            .unwrap_or(Self::model_directory(app)?.join(MODEL_ARCHIVE));
        eprintln!(
            "[wavefront] checking model archive at {}",
            model_path.display()
        );
        if !model_path.is_file() {
            eprintln!("[wavefront] model archive not found");
            if !allow_download {
                return Err("MODEL_MISSING".into());
            }
            download_model(app, &model_path)?;
        }
        let size = fs::metadata(&model_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        eprintln!("[wavefront] model archive found ({size} bytes); initializing CPU engine");
        overlay::publish(
            app,
            "loading",
            "Loading Phonon-2",
            "Initializing the local CPU engine",
        );
        phonon::engine::init_threads(None);
        let engine = Engine::load(Some(model_path), DeviceArg::Cpu, None).map_err(|error| {
            eprintln!(
                "[wavefront] CPU engine initialization failed after {:?}: {error}",
                started.elapsed()
            );
            error.to_string()
        })?;
        eprintln!(
            "[wavefront] CPU engine initialized in {:?}",
            started.elapsed()
        );
        Ok(Self { engine })
    }

    pub fn transcribe(
        &self,
        samples: Vec<f32>,
        sample_rate: usize,
    ) -> Result<Vec<phonon::text::Word>, String> {
        let samples = audio::resample(samples, sample_rate).map_err(|error| error.to_string())?;
        self.engine
            .transcribe(&samples, 4.0)
            .map_err(|error| error.to_string())
    }
}

fn download_model(app: &AppHandle, destination: &PathBuf) -> Result<(), String> {
    let parent = destination.parent().ok_or("invalid model destination")?;
    fs::create_dir_all(parent).map_err(|error| format!("Cannot create model folder: {error}"))?;
    let part = destination.with_extension("zst.part");
    let _ = fs::remove_file(&part);
    overlay::publish(
        app,
        "downloading",
        "Installing Phonon-2",
        "First run · downloading 164 MB",
    );

    let response = ureq::get(MODEL_URL).call().map_err(|error| {
        format!("Could not download the Phonon-2 model from Hugging Face: {error}")
    })?;
    let expected = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok());
    if expected.is_some_and(|size| size > MODEL_MAX_BYTES) {
        return Err("The model download is larger than expected; refusing to save it.".into());
    }

    let mut reader = response.into_reader();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|error| format!("Cannot create temporary model file: {error}"))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut written = 0u64;
    let mut last_percent = 0u64;
    let result = (|| -> Result<(), String> {
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|error| format!("Model download interrupted: {error}"))?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > MODEL_MAX_BYTES {
                return Err(
                    "The model download is larger than expected; refusing to save it.".into(),
                );
            }
            file.write_all(&buffer[..count])
                .map_err(|error| format!("Could not write the model file: {error}"))?;
            if let Some(total) = expected {
                let percent = written.saturating_mul(100) / total.max(1);
                if percent >= last_percent + 5 {
                    last_percent = percent;
                    overlay::publish(
                        app,
                        "downloading",
                        "Installing Phonon-2",
                        &format!("Downloading model · {percent}%"),
                    );
                }
            }
        }
        if let Some(total) = expected {
            if written != total {
                return Err(format!(
                    "The model download ended early ({written} of {total} bytes)."
                ));
            }
        }
        if written < 100 * 1024 * 1024 {
            return Err("The model download is incomplete; the file is unexpectedly small.".into());
        }
        file.flush()
            .map_err(|error| format!("Could not flush the model file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not finish writing the model file: {error}"))?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    fs::rename(&part, destination).map_err(|error| {
        let _ = fs::remove_file(&part);
        format!("Could not install the model file: {error}")
    })?;
    overlay::publish(
        app,
        "loading",
        "Loading Phonon-2",
        "Download complete · loading local model",
    );
    Ok(())
}
