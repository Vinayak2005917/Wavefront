use std::mem::size_of;
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

use crate::win::{self, FocusTarget, Input, InputData, KeyboardInput};

const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_UNICODE: u32 = 0x0004;
const VK_BACK: u16 = 0x08;
const WAVEFRONT_INPUT_MARKER: usize = 0x5746_524F_4E54;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertError {
    NoTarget,
    FocusChanged,
    UserInteraction,
    InputRejected,
}

#[derive(Default)]
pub struct TextWriter {
    committed_text: String,
    provisional_text: String,
    last_rendered_text: String,
    paused: bool,
}

impl TextWriter {
    pub fn apply_partial(
        &mut self,
        target: &FocusTarget,
        text: &str,
    ) -> Result<DurationMs, InsertError> {
        if self.paused {
            return Err(InsertError::UserInteraction);
        }
        let started = Instant::now();
        let (_, remove, suffix) = partial_edit(&self.provisional_text, text);
        if remove != 0 || !suffix.is_empty() {
            check_target(target)?;
            send_backspaces(remove)?;
            send_text(&suffix)?;
            self.last_rendered_text.truncate_graphemes(
                self.last_rendered_text
                    .graphemes(true)
                    .count()
                    .saturating_sub(remove),
            );
            self.last_rendered_text.push_str(&suffix);
        }
        self.provisional_text = text.to_owned();
        Ok(DurationMs(started.elapsed().as_millis() as u64))
    }

    pub fn apply_final(
        &mut self,
        target: &FocusTarget,
        text: &str,
    ) -> Result<DurationMs, InsertError> {
        let elapsed = self.apply_partial(target, text)?;
        let phrase = text.trim();
        if !phrase.is_empty() {
            self.committed_text = if self.committed_text.is_empty() {
                phrase.to_owned()
            } else {
                format!("{} {}", self.committed_text, phrase)
            };
            self.provisional_text.clear();
            check_target(target)?;
            send_text(" ")?;
            self.last_rendered_text.push(' ');
        }
        Ok(elapsed)
    }

    pub fn apply_done(&mut self, target: &FocusTarget, text: &str) -> Result<bool, InsertError> {
        let desired = text.trim();
        let current = self.last_rendered_text.trim_end();
        if current == desired {
            return Ok(false);
        }
        if self.paused {
            return Err(InsertError::UserInteraction);
        }
        check_target(target)?;
        let count = self.last_rendered_text.graphemes(true).count();
        send_backspaces(count)?;
        send_text(desired)?;
        self.last_rendered_text = desired.to_owned();
        self.committed_text = desired.to_owned();
        self.provisional_text.clear();
        Ok(true)
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.last_rendered_text.trim_end().to_owned()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DurationMs(pub u64);

trait TruncateGraphemes {
    fn truncate_graphemes(&mut self, count: usize);
}

pub fn partial_edit(old: &str, new: &str) -> (usize, usize, String) {
    let old: Vec<&str> = old.graphemes(true).collect();
    let new: Vec<&str> = new.graphemes(true).collect();
    let common = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    (common, old.len() - common, new[common..].concat())
}
impl TruncateGraphemes for String {
    fn truncate_graphemes(&mut self, count: usize) {
        let byte = self
            .grapheme_indices(true)
            .nth(count)
            .map(|(i, _)| i)
            .unwrap_or(self.len());
        self.truncate(byte);
    }
}

fn check_target(target: &FocusTarget) -> Result<(), InsertError> {
    if !target.is_valid() {
        return Err(InsertError::NoTarget);
    }
    if crate::win::user_interaction_detected() {
        return Err(InsertError::UserInteraction);
    }
    if !unsafe { win::same_focus(target) } {
        return Err(InsertError::FocusChanged);
    }
    Ok(())
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
                extra_info: WAVEFRONT_INPUT_MARKER,
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
                extra_info: WAVEFRONT_INPUT_MARKER,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grapheme_boundaries_preserve_emoji_and_composed_letters() {
        let old = "cafe\u{301} 🐈";
        let new = "cafe\u{301} 🐈!";
        let old: Vec<_> = old.graphemes(true).collect();
        let new: Vec<_> = new.graphemes(true).collect();
        let n = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        assert_eq!(n, 6);
        assert_eq!(new[n..].concat(), "!");
    }

    #[test]
    fn partial_edit_handles_extension_revision_shortening_and_punctuation() {
        assert_eq!(
            partial_edit("hello", "hello world"),
            (5, 0, " world".into())
        );
        assert_eq!(
            partial_edit("I want dictation", "I wanted writing"),
            (6, 10, "ed writing".into())
        );
        assert_eq!(partial_edit("hello world", "hello"), (5, 6, "".into()));
        assert_eq!(partial_edit("hello", "hello!"), (5, 0, "!".into()));
    }

    #[test]
    fn paused_writer_never_retries_after_external_interaction() {
        let mut writer = TextWriter::default();
        writer.pause();
        let target = FocusTarget {
            foreground: 1,
            thread_id: 1,
            process_id: 1,
            focused: 1,
        };
        assert!(matches!(
            writer.apply_partial(&target, "do not overwrite"),
            Err(InsertError::UserInteraction)
        ));
        assert!(writer.text().is_empty());
    }
}
