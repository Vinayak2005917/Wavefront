use std::ffi::c_void;
use std::mem::size_of;
use std::thread;
use std::time::Duration;

use crate::win::{self, Input, InputData, KeyboardInput};

const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_UNICODE: u32 = 0x0004;
const VK_BACK: u16 = 0x08;
const VK_CONTROL: u16 = 0x11;
const WORD_DELAY: Duration = Duration::from_millis(8);

#[derive(Debug)]
pub enum InsertError {
    NoTarget,
    FocusChanged,
    InputRejected,
}

#[derive(Default)]
pub struct TextWriter {
    typed_words: Vec<String>,
}

impl TextWriter {
    /// Apply a transcript revision using word-level deletion and insertion.
    pub fn apply(&mut self, target: isize, desired_words: &[String]) -> Result<(), InsertError> {
        let common = self
            .typed_words
            .iter()
            .zip(desired_words)
            .take_while(|(old, new)| old == new)
            .count();
        let remove_words = self.typed_words.len() - common;
        let insert_words = &desired_words[common..];
        if remove_words == 0 && insert_words.is_empty() {
            return Ok(());
        }
        check_target(target)?;

        if remove_words > 0 {
            // Wavefront leaves one separating space after its final word.
            send_inputs(&[key_input(VK_BACK, 0), key_input(VK_BACK, KEYEVENTF_KEYUP)])?;
            if common == 0 && remove_words == self.typed_words.len() {
                send_control_backspaces(remove_words.saturating_sub(1))?;
                send_backspaces(self.typed_words[0].chars().count())?;
            } else {
                send_control_backspaces(remove_words)?;
            }
        }

        for (index, word) in insert_words.iter().enumerate() {
            check_target(target)?;
            let mut chunk = String::new();
            if common > 0 || index > 0 {
                chunk.push(' ');
            }
            chunk.push_str(word);
            if index + 1 == insert_words.len() {
                chunk.push(' ');
            }
            send_text(&chunk)?;
            if index + 1 < insert_words.len() {
                thread::sleep(WORD_DELAY);
            }
        }

        self.typed_words = desired_words.to_vec();
        Ok(())
    }
}

fn check_target(target: isize) -> Result<(), InsertError> {
    if target == 0 {
        return Err(InsertError::NoTarget);
    }
    let foreground = unsafe { win::GetForegroundWindow() };
    if foreground != target as *mut c_void {
        return Err(InsertError::FocusChanged);
    }
    Ok(())
}

fn send_control_backspaces(count: usize) -> Result<(), InsertError> {
    if count == 0 {
        return Ok(());
    }
    let mut inputs = Vec::with_capacity(count * 4);
    for _ in 0..count {
        inputs.push(key_input(VK_CONTROL, 0));
        inputs.push(key_input(VK_BACK, 0));
        inputs.push(key_input(VK_BACK, KEYEVENTF_KEYUP));
        inputs.push(key_input(VK_CONTROL, KEYEVENTF_KEYUP));
    }
    send_inputs(&inputs)
}

fn send_backspaces(count: usize) -> Result<(), InsertError> {
    if count == 0 {
        return Ok(());
    }
    let mut inputs = Vec::with_capacity(count * 2);
    for _ in 0..count {
        inputs.push(key_input(VK_BACK, 0));
        inputs.push(key_input(VK_BACK, KEYEVENTF_KEYUP));
    }
    send_inputs(&inputs)
}

fn send_text(text: &str) -> Result<(), InsertError> {
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        inputs.push(unicode_input(unit, KEYEVENTF_UNICODE));
        inputs.push(unicode_input(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send_inputs(&inputs)
}

fn send_inputs(inputs: &[Input]) -> Result<(), InsertError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe {
        win::SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<Input>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(InsertError::InputRejected)
    }
}

fn key_input(code: u16, flags: u32) -> Input {
    Input {
        input_type: INPUT_KEYBOARD,
        padding: 0,
        data: InputData {
            keyboard: KeyboardInput {
                vk: code,
                scan: 0,
                flags,
                time: 0,
                extra_info: 0,
            },
        },
    }
}

fn unicode_input(code: u16, flags: u32) -> Input {
    Input {
        input_type: INPUT_KEYBOARD,
        padding: 0,
        data: InputData {
            keyboard: KeyboardInput {
                vk: 0,
                scan: code,
                flags,
                time: 0,
                extra_info: 0,
            },
        },
    }
}
