use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::{dictation::Command, overlay, win};

static THREAD_ID: AtomicU32 = AtomicU32::new(0);
const HOTKEY_ID: i32 = 1;
const DEFAULT_SHORTCUT: &str = "Ctrl + Q";

#[derive(Clone)]
struct Shortcut {
    modifiers: u32,
    key: u32,
    display: String,
}

enum UpdateRequest {
    Set {
        shortcut: Shortcut,
        reply: SyncSender<Result<(), String>>,
    },
    Suspend {
        reply: SyncSender<Result<(), String>>,
    },
    Resume {
        reply: SyncSender<Result<(), String>>,
    },
}

pub struct Controller {
    updates: Sender<UpdateRequest>,
}

fn shortcut_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|directory| directory.join("dictation-shortcut.txt"))
        .map_err(|error| error.to_string())
}

pub fn saved_shortcut(app: &AppHandle) -> String {
    let Some(path) = shortcut_file(app).ok() else {
        return DEFAULT_SHORTCUT.to_owned();
    };
    let Some(value) = fs::read_to_string(&path).ok() else {
        return DEFAULT_SHORTCUT.to_owned();
    };
    let value = value.trim();
    let Ok(parsed) = Shortcut::parse(value) else {
        return DEFAULT_SHORTCUT.to_owned();
    };

    // Replace defaults from earlier builds while preserving shortcuts the user chose.
    if matches!(
        parsed.display.as_str(),
        "Win + Shift + D" | "Ctrl + Shift + D"
    ) {
        let _ = fs::write(path, DEFAULT_SHORTCUT);
        DEFAULT_SHORTCUT.to_owned()
    } else {
        parsed.display
    }
}

pub fn save_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let path = shortcut_file(app)?;
    let parent = path.parent().ok_or("invalid shortcut settings path")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    fs::write(path, shortcut).map_err(|error| format!("Could not save shortcut: {error}"))
}

impl Shortcut {
    fn parse(value: &str) -> Result<Self, String> {
        let mut modifiers = 0u32;
        let mut key = None;
        let mut key_name = String::new();
        for part in value
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match part.to_ascii_lowercase().as_str() {
                "win" | "windows" => modifiers |= win::MOD_WIN,
                "ctrl" | "control" => modifiers |= win::MOD_CONTROL,
                "alt" => modifiers |= win::MOD_ALT,
                "shift" => modifiers |= win::MOD_SHIFT,
                _ => {
                    if key.is_some() {
                        return Err("Enter one key with modifiers, such as Win + Shift + D.".into());
                    }
                    let (virtual_key, canonical) = parse_key(part).ok_or_else(|| {
                        "Use a letter, number, F1–F24, Space, or a named navigation key.".to_owned()
                    })?;
                    key = Some(virtual_key);
                    key_name = canonical;
                }
            }
        }
        let key = key.ok_or("Add a key to the shortcut, such as Win + Shift + D.")?;
        if modifiers == 0 {
            return Err(
                "The shortcut must include at least one modifier: Win, Ctrl, Alt, or Shift.".into(),
            );
        }
        let mut names = Vec::new();
        if modifiers & win::MOD_WIN != 0 {
            names.push("Win");
        }
        if modifiers & win::MOD_CONTROL != 0 {
            names.push("Ctrl");
        }
        if modifiers & win::MOD_ALT != 0 {
            names.push("Alt");
        }
        if modifiers & win::MOD_SHIFT != 0 {
            names.push("Shift");
        }
        names.push(&key_name);
        Ok(Self {
            modifiers,
            key,
            display: names.join(" + "),
        })
    }
}

fn parse_key(value: &str) -> Option<(u32, String)> {
    let upper = value.trim().to_ascii_uppercase();
    if upper.len() == 1 {
        let byte = upper.as_bytes()[0];
        if byte.is_ascii_alphanumeric() {
            return Some((u32::from(byte), upper));
        }
    }
    let key = match upper.as_str() {
        "SPACE" => (0x20, "Space"),
        "TAB" => (0x09, "Tab"),
        "ENTER" | "RETURN" => (0x0D, "Enter"),
        "ESC" | "ESCAPE" => (0x1B, "Escape"),
        "BACKSPACE" => (0x08, "Backspace"),
        "DELETE" | "DEL" => (0x2E, "Delete"),
        "INSERT" | "INS" => (0x2D, "Insert"),
        "HOME" => (0x24, "Home"),
        "END" => (0x23, "End"),
        "PAGEUP" => (0x21, "PageUp"),
        "PAGEDOWN" => (0x22, "PageDown"),
        "LEFT" => (0x25, "Left"),
        "UP" => (0x26, "Up"),
        "RIGHT" => (0x27, "Right"),
        "DOWN" => (0x28, "Down"),
        _ => {
            let number = upper.strip_prefix('F')?.parse::<u32>().ok()?;
            if !(1..=24).contains(&number) {
                return None;
            }
            return Some((0x70 + number - 1, format!("F{number}")));
        }
    };
    Some((key.0, key.1.to_owned()))
}

pub fn start(app: AppHandle, commands: Sender<Command>, initial: &str) -> Controller {
    let initial =
        Shortcut::parse(initial).unwrap_or_else(|_| Shortcut::parse(DEFAULT_SHORTCUT).unwrap());
    let (updates, update_rx) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("wavefront-hotkey".into())
        .spawn(move || unsafe {
            THREAD_ID.store(win::GetCurrentThreadId(), Ordering::SeqCst);
            let mut msg = std::mem::MaybeUninit::<win::Msg>::zeroed().assume_init();
            // Make sure this thread owns a message queue before registering a thread hotkey.
            win::PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, win::PM_NOREMOVE);
            let mut active = initial;
            if register(&active).is_err() {
                let error = win::GetLastError();
                overlay::publish(
                    &app,
                    "hotkey_error",
                    "Shortcut unavailable",
                    &format!("{} could not be registered (Windows error {error})", active.display),
                );
                THREAD_ID.store(0, Ordering::SeqCst);
                return;
            }

            let _ = app.emit("hotkey-ready", ());
            let mut suspended = false;
            while win::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == win::WM_APP_HOTKEY_UPDATE || msg.message == win::WM_HOTKEY {
                    let mut processed_update = false;
                    while let Ok(request) = update_rx.try_recv() {
                        processed_update = true;
                        match request {
                            UpdateRequest::Set { shortcut, reply } => {
                                if suspended {
                                    active = shortcut;
                                    let _ = reply.send(Ok(()));
                                } else {
                                    let previous = active.clone();
                                    win::UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
                                    match register(&shortcut) {
                                        Ok(()) => {
                                            active = shortcut;
                                            let _ = reply.send(Ok(()));
                                            let _ = app.emit("hotkey-changed", active.display.clone());
                                        }
                                        Err(()) => {
                                            let error = win::GetLastError();
                                            let _ = register(&previous);
                                            active = previous;
                                            let _ = reply.send(Err(format!(
                                                "That shortcut is already in use or Windows rejected it (error {error})."
                                            )));
                                        }
                                    }
                                }
                            }
                            UpdateRequest::Suspend { reply } => {
                                if !suspended {
                                    win::UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
                                    suspended = true;
                                }
                                let _ = reply.send(Ok(()));
                            }
                            UpdateRequest::Resume { reply } => {
                                if suspended {
                                    match register(&active) {
                                        Ok(()) => {
                                            suspended = false;
                                            let _ = reply.send(Ok(()));
                                            let _ = app.emit("hotkey-changed", active.display.clone());
                                        }
                                        Err(()) => {
                                            let error = win::GetLastError();
                                            let _ = reply.send(Err(format!(
                                                "Could not restore {} (Windows error {error}).",
                                                active.display
                                            )));
                                        }
                                    }
                                } else {
                                    let _ = reply.send(Ok(()));
                                }
                            }
                        }
                    }
                    if !processed_update
                        && !suspended
                        && msg.message == win::WM_HOTKEY
                        && msg.w_param == HOTKEY_ID as usize
                    {
                        let foreground = win::GetForegroundWindow() as isize;
                        let _ = commands.send(Command::Toggle { target: foreground });
                    }
                } else {
                    win::TranslateMessage(&msg);
                    win::DispatchMessageW(&msg);
                }
            }
            win::UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
            THREAD_ID.store(0, Ordering::SeqCst);
        });
    Controller { updates }
}

fn register(shortcut: &Shortcut) -> Result<(), ()> {
    let success = unsafe {
        win::RegisterHotKey(
            std::ptr::null_mut(),
            HOTKEY_ID,
            shortcut.modifiers | win::MOD_NOREPEAT,
            shortcut.key,
        )
    };
    if success == 0 { Err(()) } else { Ok(()) }
}

pub fn set(controller: &Controller, value: &str) -> Result<String, String> {
    let shortcut = Shortcut::parse(value)?;
    let display = shortcut.display.clone();
    request_update(controller, |reply| UpdateRequest::Set { shortcut, reply })?;
    Ok(display)
}

pub fn suspend(controller: &Controller) -> Result<(), String> {
    request_update(controller, |reply| UpdateRequest::Suspend { reply })
}

pub fn resume(controller: &Controller) -> Result<(), String> {
    request_update(controller, |reply| UpdateRequest::Resume { reply })
}

fn request_update(
    controller: &Controller,
    request: impl FnOnce(SyncSender<Result<(), String>>) -> UpdateRequest,
) -> Result<(), String> {
    let thread_id = THREAD_ID.load(Ordering::SeqCst);
    if thread_id == 0 {
        return Err(
            "The shortcut listener is unavailable. Restart Wavefront and try again.".into(),
        );
    }
    let (reply, response) = mpsc::sync_channel(1);
    controller
        .updates
        .send(request(reply))
        .map_err(|_| {
            "The shortcut listener is unavailable. Restart Wavefront and try again.".to_owned()
        })?;
    if unsafe { win::PostThreadMessageW(thread_id, win::WM_APP_HOTKEY_UPDATE, 0, 0) } == 0 {
        return Err(
            "The shortcut listener is unavailable. Restart Wavefront and try again.".into(),
        );
    }
    response
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "Timed out while updating the shortcut.".to_owned())??;
    Ok(())
}

pub fn stop() {
    let id = THREAD_ID.load(Ordering::SeqCst);
    if id != 0 {
        unsafe {
            win::PostThreadMessageW(id, win::WM_QUIT, 0, 0);
        }
    }
}
