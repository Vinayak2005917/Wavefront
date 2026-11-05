use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::{
    audio::AudioSession,
    overlay,
    recognizer::{AsrEngine, AsrStream, ClientEvent, RecognizerSettings},
    text_injection::{self, InsertError, TextWriter},
    transcript::{ServerEvent, StreamingTranscript},
    win::{self, FocusTarget, InteractionHooks},
};

const WAVEFORM_POINTS: usize = 144;

pub enum Command {
    Toggle { target: isize },
    ReloadModel,
    Shutdown,
}

struct ActiveSession {
    id: u64,
    focus: FocusTarget,
    audio: Option<AudioSession>,
    client: AsrStream,
    transcript: StreamingTranscript,
    writer: TextWriter,
    _hooks: InteractionHooks,
    started: Instant,
    first_audio: bool,
    ending: bool,
    done_received: bool,
    insertion_paused: bool,
}

pub fn start(app: AppHandle) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let worker_app = app.clone();
    if let Err(e) = thread::Builder::new()
        .name("wavefront-dictation".into())
        .spawn(move || run(worker_app, rx))
    {
        eprintln!("[wavefront] could not start dictation worker: {e}");
        overlay::publish(&app, "asr_error", "Recognizer unavailable", &e.to_string());
    }
    tx
}

fn run(app: AppHandle, commands: Receiver<Command>) {
    let mut settings = match RecognizerSettings::load(&app) {
        Ok(settings) => settings,
        Err(error) => {
            overlay::publish(&app, "asr_error", "Model setup failed", &error);
            return;
        }
    };
    let mut engine = load_engine(&app, &settings);
    let mut active = None;
    let mut reload_after_session = false;
    let mut next_id = 1u64;

    loop {
        if let Some(session) = active.as_mut() {
            pump_session(&app, session);
            if session.ending && session.done_received {
                finish_record(&app, active.take().expect("active session"));
                if reload_after_session {
                    settings = RecognizerSettings::load(&app).unwrap_or(settings);
                    engine = load_engine(&app, &settings);
                    reload_after_session = false;
                }
                ready_state(&app, engine.as_ref());
            }
            match commands.try_recv() {
                Ok(Command::Toggle { .. }) if active.as_ref().is_some_and(|s| !s.ending) => {
                    stop_session(&app, active.as_mut().expect("active session"));
                }
                Ok(Command::ReloadModel) => {
                    if let Some(session) = active.as_mut() {
                        if !session.ending {
                            stop_session(&app, session);
                        }
                        reload_after_session = true;
                    }
                }
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => {
                    if let Some(session) = active.as_mut().filter(|s| !s.ending) {
                        stop_session(&app, session);
                    }
                    let deadline = Instant::now() + Duration::from_secs(3);
                    while Instant::now() < deadline
                        && active.as_ref().is_some_and(|s| !s.done_received)
                    {
                        if let Some(session) = active.as_mut() {
                            pump_session(&app, session);
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                    break;
                }
                _ => {}
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Toggle { target }) => {
                if let Some(engine) = engine.as_ref() {
                    active = start_session(&app, engine, target, next_id);
                    next_id = next_id.wrapping_add(1).max(1);
                } else {
                    overlay::publish(
                        &app,
                        "asr_error",
                        "No recognizer is ready",
                        "Select Reload model after checking the local runtime, model folder, or network connection.",
                    );
                }
            }
            Ok(Command::ReloadModel) => {
                settings = RecognizerSettings::load(&app).unwrap_or(settings);
                engine = load_engine(&app, &settings);
            }
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    overlay::publish(
        &app,
        "stopped",
        "Dictation stopped",
        "The local recognizer is stopped",
    );
    overlay::publish_waveform(&app, &vec![0.0; WAVEFORM_POINTS]);
    overlay::hide(&app);
}

fn load_engine(app: &AppHandle, settings: &RecognizerSettings) -> Option<AsrEngine> {
    match AsrEngine::load(app, settings) {
        Ok(engine) => {
            let (message, detail) = engine.ready_state();
            overlay::publish(app, "ready", message, &detail);
            Some(engine)
        }
        Err(error) => {
            eprintln!("[wavefront] recognizer setup failed: {error}");
            overlay::publish(
                app,
                "asr_error",
                "Speech recognizer needs attention",
                &error,
            );
            None
        }
    }
}

fn start_session(
    app: &AppHandle,
    engine: &AsrEngine,
    target: isize,
    id: u64,
) -> Option<ActiveSession> {
    let Some(focus) = win::capture_focus(target) else {
        overlay::publish(
            app,
            "focus_changed",
            "Text insertion paused",
            "Wavefront could not identify the focused control. Focus a text field and try again.",
        );
        return None;
    };
    let hooks = match InteractionHooks::start() {
        Ok(hooks) => hooks,
        Err(error) => {
            overlay::publish(app, "error", "Text safety monitor unavailable", &error);
            return None;
        }
    };
    let client = match engine.create_stream() {
        Ok(client) => client,
        Err(error) => {
            overlay::publish(app, "asr_error", "Could not start recognizer", &error);
            return None;
        }
    };
    let audio = match AudioSession::open() {
        Ok(audio) => audio,
        Err(error) => {
            overlay::publish(app, "error", "Microphone unavailable", &error);
            return None;
        }
    };
    overlay::publish_waveform(app, &vec![0.0; WAVEFORM_POINTS]);
    overlay::show(app);
    overlay::publish(
        app,
        "listening",
        "Listening",
        &format!(
            "Streaming microphone audio to the local {} model",
            engine.model_name()
        ),
    );
    eprintln!(
        "[wavefront] microphone session started; mono float32 input={} Hz (passed at native rate)",
        audio.sample_rate()
    );
    Some(ActiveSession {
        id,
        focus,
        audio: Some(audio),
        client,
        transcript: StreamingTranscript::default(),
        writer: TextWriter::default(),
        _hooks: hooks,
        started: Instant::now(),
        first_audio: false,
        ending: false,
        done_received: false,
        insertion_paused: false,
    })
}

fn stop_session(app: &AppHandle, session: &mut ActiveSession) {
    if session.ending {
        return;
    }
    overlay::publish(
        app,
        "finalizing",
        "Finalizing speech",
        "Flushing the remaining audio through the local recognizer",
    );
    if let Some(mut audio) = session.audio.take() {
        let sample_rate = audio.sample_rate();
        let update = audio.stop();
        if let Some(samples) = update.waveform.as_deref() {
            overlay::publish_waveform(app, samples);
        }
        if update.overrun_samples > 0 {
            overlay::publish(
                app,
                "asr_error",
                "Microphone capture overrun",
                "Some microphone samples were lost before streaming. The incomplete session was finalized.",
            );
        }
        for frame in update.frames {
            if let Err(error) = session.client.send_audio(&frame, sample_rate) {
                overlay::publish(app, "asr_error", "Audio stream stopped", &error);
            }
        }
    }
    if let Err(error) = session.client.finish() {
        overlay::publish(app, "asr_error", "Could not finalize stream", &error);
        session.done_received = true;
    }
    session.ending = true;
}

fn pump_session(app: &AppHandle, session: &mut ActiveSession) {
    if !session.insertion_paused && win::user_interaction_detected() {
        session.writer.pause();
        session.insertion_paused = true;
        publish_insert_error(app, InsertError::UserInteraction);
    }
    if let Some(audio) = session.audio.as_mut() {
        let sample_rate = audio.sample_rate();
        let update = audio.poll();
        if let Some(samples) = update.waveform.as_deref() {
            overlay::publish_waveform(app, samples);
        }
        let overrun = update.overrun_samples;
        for frame in update.frames {
            if !session.first_audio {
                session.first_audio = true;
                eprintln!("[wavefront] first audio frame reached recognizer");
            }
            if let Err(error) = session.client.send_audio(&frame, sample_rate) {
                stop_session(app, session);
                overlay::publish(app, "asr_error", "Audio stream stopped", &error);
                break;
            }
        }
        if overrun > 0 && !session.ending {
            stop_session(app, session);
            overlay::publish(
                app,
                "asr_error",
                "Microphone capture overrun",
                "Some microphone samples were lost before streaming. The incomplete session was finalized.",
            );
        }
    }

    loop {
        match session.client.try_event() {
            Ok(ClientEvent::Server(ServerEvent::Partial { text })) => {
                session.transcript.partial(text.clone());
                if !session.insertion_paused {
                    let result = session.writer.apply_partial(&session.focus, &text);
                    apply_insert(app, session, result);
                }
            }
            Ok(ClientEvent::Server(ServerEvent::Final { text, segment })) => {
                if !session.insertion_paused {
                    let result = session.writer.apply_final(&session.focus, &text);
                    apply_insert(app, session, result);
                }
                session.transcript.finalize(text, segment);
            }
            Ok(ClientEvent::Server(ServerEvent::Done { text })) => {
                let mismatch = session.transcript.reconcile_done(&text);
                if mismatch && !session.insertion_paused {
                    match session.writer.apply_done(&session.focus, &text) {
                        Ok(true) => eprintln!(
                            "[wavefront] reconciled final transcript while target ownership was valid"
                        ),
                        Ok(false) => {}
                        Err(error) => {
                            session.writer.pause();
                            session.insertion_paused = true;
                            publish_insert_error(app, error);
                        }
                    }
                }
                session.done_received = true;
                session.ending = true;
                session.audio.take();
            }
            Ok(ClientEvent::Server(ServerEvent::Error { message })) => {
                overlay::publish(app, "asr_error", "Recognition failed", &message);
                session.done_received = true;
                session.ending = true;
                session.audio.take();
            }
            Ok(ClientEvent::Server(ServerEvent::Unknown)) => {}
            Ok(ClientEvent::Closed) => {
                if !session.done_received {
                    overlay::publish(
                        app,
                        "asr_error",
                        "Recognition stream closed",
                        "The local recognizer ended before finalizing the transcript.",
                    );
                    session.ending = true;
                    session.audio.take();
                }
                session.done_received = true;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                session.done_received = true;
                break;
            }
        }
    }
}

fn apply_insert(
    app: &AppHandle,
    session: &mut ActiveSession,
    result: Result<text_injection::DurationMs, InsertError>,
) {
    match result {
        Ok(time) => eprintln!("[wavefront] text injection completed in {} ms", time.0),
        Err(error) => {
            session.writer.pause();
            session.insertion_paused = true;
            publish_insert_error(app, error);
        }
    }
}

fn finish_record(app: &AppHandle, session: ActiveSession) {
    let text = session.transcript.assembled();
    if !text.is_empty() {
        overlay::publish_transcript(app, &text);
    }
    eprintln!(
        "[wavefront] session {} completed in {:?}",
        session.id,
        session.started.elapsed()
    );
}

fn ready_state(app: &AppHandle, engine: Option<&AsrEngine>) {
    if let Some(engine) = engine {
        let (message, detail) = engine.ready_state();
        overlay::publish(app, "ready", message, &detail);
        let app = app.clone();
        let _ = thread::Builder::new()
            .name("wavefront-pill-dismiss".into())
            .spawn(move || {
                thread::sleep(Duration::from_millis(350));
                if overlay::current_state()
                    .get("state")
                    .and_then(|v| v.as_str())
                    == Some("ready")
                {
                    overlay::hide(&app);
                }
            });
    } else {
        overlay::publish(
            app,
            "asr_error",
            "Recognizer unavailable",
            "Reload the model after fixing the local model setup.",
        );
    }
}

fn publish_insert_error(app: &AppHandle, error: InsertError) {
    let detail = match error {
        InsertError::FocusChanged => {
            "The focused control changed. Wavefront preserved inserted text and paused further edits."
        }
        InsertError::UserInteraction => {
            "You interacted with the keyboard or mouse. Wavefront preserved inserted text and paused further edits."
        }
        InsertError::NoTarget => {
            "Wavefront could not verify the original focused text control. Text insertion is paused."
        }
        InsertError::InputRejected => {
            "Windows rejected synthetic text input. Wavefront will not retry to avoid duplicates."
        }
    };
    overlay::publish(app, "focus_changed", "Text insertion paused", detail);
}

