use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use tauri::AppHandle;

use crate::{
    audio::AudioSession,
    overlay,
    recognizer::Recognizer,
    text_injection::{self, InsertError},
    transcript::{TranscriptManager, WordHypothesis},
};

const WAVEFORM_POINTS: usize = 144;
const UPDATE_INTERVAL: Duration = Duration::from_millis(300);

pub enum Command {
    Toggle { target: isize, test: bool },
    StartTest { target: isize },
    InstallModel { path: PathBuf },
    Shutdown,
}

struct ActiveSession {
    id: u64,
    target: Option<isize>,
    audio: AudioSession,
    last_update: std::time::Instant,
    revision: u64,
    test: bool,
}

struct RecognitionRequest {
    session_id: u64,
    target: isize,
    revision: u64,
    sample_rate: usize,
    window_start: f32,
    window_end: f32,
    samples: Vec<f32>,
    test: bool,
}

struct RecognitionResult {
    session_id: u64,
    target: isize,
    revision: u64,
    window_start: f32,
    window_end: f32,
    result: Result<Vec<WordHypothesis>, String>,
    test: bool,
}

struct SessionTranscript {
    manager: TranscriptManager,
    writer: text_injection::TextWriter,
    pending: usize,
    ended: bool,
    latest_revision: u64,
    test: bool,
    test_started_ms: u64,
}
pub fn start(app: AppHandle) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let worker_app = app.clone();
    if let Err(error) = thread::Builder::new()
        .name("wavefront-audio".into())
        .spawn(move || run(worker_app, rx))
    {
        eprintln!("[wavefront] failed to start audio/model worker: {error}");
        overlay::publish(&app, "error", "Model setup unavailable", &error.to_string());
    }
    tx
}

fn run(app: AppHandle, commands: Receiver<Command>) {
    eprintln!("[wavefront] audio/model worker started");
    let recognizer = match load_model_or_wait(&app, &commands) {
        Some(recognizer) => recognizer,
        None => return,
    };

    let (recognition_tx, recognition_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    if let Err(error) = thread::Builder::new()
        .name("wavefront-recognizer".into())
        .spawn(move || recognition_worker(recognizer, recognition_rx, result_tx))
    {
        overlay::publish(&app, "error", "Recognition unavailable", &error.to_string());
        return;
    }

    let mut active: Option<ActiveSession> = None;
    let mut pending_recognitions = 0usize;
    let mut session_transcripts = HashMap::<u64, SessionTranscript>::new();
    let mut next_session_id = 1u64;
    let mut blocked_session = None;
    loop {
        while let Ok(result) = result_rx.try_recv() {
            pending_recognitions = pending_recognitions.saturating_sub(1);
            handle_result(
                &app,
                result,
                &mut active,
                &mut blocked_session,
                &mut session_transcripts,
            );

            if active.is_none() && pending_recognitions == 0 {
                let state = overlay::current_state();
                let state = state.get("state").and_then(|value| value.as_str());
                if state != Some("error") && state != Some("focus_changed") {
                    hide_when_idle(&app);
                }
            }
        }

        if active.is_none() {
            match commands.recv_timeout(Duration::from_millis(25)) {
                Ok(Command::Toggle { target, test }) => {
                    start_session(
                        &app,
                        target,
                        &mut active,
                        &mut next_session_id,
                        &mut session_transcripts,
                        test,
                    );
                }
                Ok(Command::StartTest { target }) => {
                    start_session(&app, target, &mut active, &mut next_session_id, &mut session_transcripts, true);
                }
                Ok(Command::InstallModel { .. }) => {}
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            continue;
        }

        match commands.try_recv() {
            Ok(Command::InstallModel { .. }) => {}
            Ok(Command::StartTest { target }) => {
                if let Some(session) = active.take() {
                    stop_session(&app, &recognition_tx, session, &mut pending_recognitions, &mut session_transcripts);
                }
                start_session(&app, target, &mut active, &mut next_session_id, &mut session_transcripts, true);
                continue;
            }
            Ok(Command::Toggle { .. }) => {
                if let Some(session) = active.take() {
                    stop_session(
                        &app,
                        &recognition_tx,
                        session,
                        &mut pending_recognitions,
                        &mut session_transcripts,
                    );
                }
                if pending_recognitions == 0 {
                    hide_when_idle(&app);
                } else {
                    overlay::publish(
                        &app,
                        "processing",
                        "Finishing speech",
                        "Transcribing the last phrase",
                    );
                }
                continue;
            }
            Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => {
                if let Some(session) = active.take() {
                    stop_session(
                        &app,
                        &recognition_tx,
                        session,
                        &mut pending_recognitions,
                        &mut session_transcripts,
                    );
                }
                break;
            }
            Err(TryRecvError::Empty) => {}
        }

        if let Some(session) = active.as_mut() {
            let update = session.audio.poll();
            if let Some(samples) = update.waveform.as_deref() {
                overlay::publish_waveform(&app, samples);
            }
            if session.audio.has_speech()
                && session.audio.window_len() >= session.audio.sample_rate() * 4
                && session.last_update.elapsed() >= UPDATE_INTERVAL
                && session_transcripts
                    .get(&session.id)
                    .is_some_and(|s| s.pending == 0)
            {
                queue_window(
                    &app,
                    &recognition_tx,
                    session,
                    &mut pending_recognitions,
                    &mut session_transcripts,
                );
            }
        }

        thread::sleep(Duration::from_millis(20));
    }

    overlay::publish(&app, "stopped", "Dictation stopped", "Ready to dictate");
    overlay::publish_waveform(&app, &vec![0.0; WAVEFORM_POINTS]);
    overlay::hide(&app);
}

fn load_model_or_wait(app: &AppHandle, commands: &Receiver<Command>) -> Option<Recognizer> {
    eprintln!("[wavefront] checking configured local model");
    match Recognizer::load(app, None, false) {
        Ok(recognizer) => {
            eprintln!("[wavefront] local model loaded successfully");
            overlay::publish(
                app,
                "ready",
                "Ready to dictate",
                "Model ready on this device",
            );
            return Some(recognizer);
        }
        Err(error) if error == "MODEL_MISSING" => {
            eprintln!("[wavefront] local model archive is missing; waiting for install");
            overlay::publish(
                app,
                "model_missing",
                "Phonon-2 is not installed",
                "Choose where to save the model, then select Download model.",
            );
        }
        Err(error) => {
            eprintln!("[wavefront] local model check/load failed: {error}");
            overlay::publish(app, "model_error", "Phonon-2 setup failed", &error)
        }
    }

    loop {
        match commands.recv() {
            Ok(Command::InstallModel { path }) => {
                eprintln!("[wavefront] install requested at {}", path.display());
                match Recognizer::load(app, Some(path), true) {
                    Ok(recognizer) => {
                        eprintln!("[wavefront] installed model loaded successfully");
                        overlay::publish(
                            app,
                            "ready",
                            "Ready to dictate",
                            "Model ready on this device",
                        );
                        return Some(recognizer);
                    }
                    Err(error) => {
                        eprintln!("[wavefront] model install/load failed: {error}");
                        overlay::publish(app, "model_error", "Phonon-2 setup failed", &error)
                    }
                }
            }
            Ok(Command::Shutdown) | Err(_) => return None,
            Ok(Command::Toggle { .. }) | Ok(Command::StartTest { .. }) => {}
        }
    }
}

fn start_session(
    app: &AppHandle,
    target: isize,
    active: &mut Option<ActiveSession>,
    next_session_id: &mut u64,
    session_transcripts: &mut HashMap<u64, SessionTranscript>,
    test: bool,
) {
    match AudioSession::open() {
        Ok(audio) => {
            let id = *next_session_id;
            *next_session_id = next_session_id.wrapping_add(1).max(1);
            *active = Some(ActiveSession {
                id,
                target: (target != 0).then_some(target),
                audio,
                last_update: std::time::Instant::now() - UPDATE_INTERVAL,
                revision: 0,
                test,
            });
            session_transcripts.insert(
                id,
                SessionTranscript {
                    manager: TranscriptManager::default(),
                    writer: text_injection::TextWriter::default(),
                    pending: 0,
                    ended: false,
                    latest_revision: 0,
                    test,
                    test_started_ms: now_ms(),
                },
            );
            overlay::publish_waveform(app, &vec![0.0; WAVEFORM_POINTS]);
            overlay::show(app);
            overlay::publish(app, "listening", "Listening", "Microphone active");
        }
        Err(error) => {
            overlay::publish(app, "error", "Microphone unavailable", &error);
            overlay::show(app);
        }
    }
}

fn queue_window(
    app: &AppHandle,
    tx: &Sender<RecognitionRequest>,
    session: &mut ActiveSession,
    pending: &mut usize,
    session_transcripts: &mut HashMap<u64, SessionTranscript>,
) {
    if let Some(target) = session.target {
        let samples = session.audio.window();
        session.revision = session.revision.wrapping_add(1);
        let request = RecognitionRequest {
            session_id: session.id,
            target,
            revision: session.revision,
            sample_rate: session.audio.sample_rate(),
            window_start: session.audio.window_start_seconds(),
            window_end: session.audio.window_start_seconds()
                + samples.len() as f32 / session.audio.sample_rate() as f32,
            samples,
            test: session.test,
        };
        if tx.send(request).is_ok() {
            *pending += 1;
            if let Some(transcript) = session_transcripts.get_mut(&session.id) {
                transcript.pending += 1;
            }
            session.last_update = std::time::Instant::now();
        } else {
            overlay::publish(
                app,
                "error",
                "Recognition unavailable",
                "The local recognition worker stopped.",
            );
        }
    }
}

fn stop_session(
    app: &AppHandle,
    tx: &Sender<RecognitionRequest>,
    mut session: ActiveSession,
    pending: &mut usize,
    session_transcripts: &mut HashMap<u64, SessionTranscript>,
) {
    overlay::publish(
        app,
        "processing",
        "Finishing speech",
        "Transcribing the last phrase",
    );
    let update = session.audio.stop();
    if let Some(samples) = update.waveform.as_deref() {
        overlay::publish_waveform(app, samples);
    }
    if session.audio.has_speech() && session.target.is_some() {
        queue_window(app, tx, &mut session, pending, session_transcripts);
    }
    if let Some(transcript) = session_transcripts.get_mut(&session.id) {
        transcript.ended = true;
    }
    finalize_session_transcript(app, session.id, session_transcripts);
}

fn recognition_worker(
    recognizer: Recognizer,
    requests: Receiver<RecognitionRequest>,
    results: Sender<RecognitionResult>,
) {
    while let Ok(request) = requests.recv() {
        let window_start = request.window_start;
        let result = recognizer
            .transcribe(request.samples, request.sample_rate)
            .map(|words| {
                words
                    .into_iter()
                    .map(|word| WordHypothesis {
                        text: word.word,
                        start: window_start + word.start,
                        end: window_start + word.end,
                    })
                    .collect()
            });
        if results
            .send(RecognitionResult {
                session_id: request.session_id,
                target: request.target,
                revision: request.revision,
                window_start: request.window_start,
                window_end: request.window_end,
                result,
                test: request.test,
            })
            .is_err()
        {
            break;
        }
    }
}

fn handle_result(
    app: &AppHandle,
    result: RecognitionResult,
    active: &mut Option<ActiveSession>,
    blocked_session: &mut Option<u64>,
    session_transcripts: &mut HashMap<u64, SessionTranscript>,
) {
    if let Some(session) = session_transcripts.get_mut(&result.session_id) {
        session.pending = session.pending.saturating_sub(1);
    }
    if *blocked_session == Some(result.session_id) {
        finalize_session_transcript(app, result.session_id, session_transcripts);
        return;
    }

    if session_transcripts
        .get(&result.session_id)
        .is_some_and(|session| result.revision <= session.latest_revision)
    {
        finalize_session_transcript(app, result.session_id, session_transcripts);
        return;
    }

    match result.result {
        Ok(words) => {
            eprintln!(
                "[wavefront] transcription completed ({} words)",
                words.len()
            );
            let Some(session) = session_transcripts.get_mut(&result.session_id) else {
                return;
            };
            session.latest_revision = result.revision;
            if result.test {
                overlay::publish_test_update(app, result.session_id, result.revision, result.window_start, result.window_end, &words);
            }
            let desired = session
                .manager
                .update(&words, result.window_start, result.window_end);
            if let Err(error) = session.writer.apply(result.target, &desired) {
                *blocked_session = Some(result.session_id);
                if let Some(session) = active.as_mut() {
                    if session.id == result.session_id {
                        session.target = None;
                    }
                }
                publish_insert_error(app, error);
            }
        }
        Err(error) => {
            eprintln!("[wavefront] transcription failed: {error}");
            overlay::publish(app, "error", "Recognition failed", &error);
        }
    }
    finalize_session_transcript(app, result.session_id, session_transcripts);
}

fn finalize_session_transcript(
    app: &AppHandle,
    session_id: u64,
    sessions: &mut HashMap<u64, SessionTranscript>,
) {
    let is_complete = sessions
        .get(&session_id)
        .is_some_and(|session| session.ended && session.pending == 0);
    if !is_complete {
        return;
    }
    if let Some(session) = sessions.remove(&session_id) {
        let text = session.manager.final_text();
        if session.test {
            overlay::publish_test_complete(app, session_id, session.test_started_ms, &text);
        }
        if !text.is_empty() {
            overlay::publish_transcript(app, &text);
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
}

fn hide_when_idle(app: &AppHandle) {
    overlay::publish(
        app,
        "ready",
        "Ready to dictate",
        "Model ready on this device",
    );
    let app = app.clone();
    let _ = thread::Builder::new()
        .name("wavefront-pill-dismiss".into())
        .spawn(move || {
            thread::sleep(Duration::from_millis(240));
            if overlay::current_state()
                .get("state")
                .and_then(|value| value.as_str())
                == Some("ready")
            {
                overlay::hide(&app);
            }
        });
}

fn publish_insert_error(app: &AppHandle, error: InsertError) {
    let detail = match error {
        InsertError::FocusChanged => {
            "The original text field lost focus. Dictation continues, but no more text will be sent to avoid typing in another app."
        }
        InsertError::NoTarget => "No focused text window was captured when dictation started.",
        InsertError::InputRejected => {
            "Windows rejected synthetic text input. This phrase was not retried to avoid duplicates."
        }
    };
    overlay::publish(app, "focus_changed", "Text insertion paused", detail);
}
