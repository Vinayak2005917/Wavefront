#[derive(Clone, Debug)]
pub struct WordHypothesis {
    pub text: String,
    pub start: f32,
    pub end: f32,
}

#[derive(Clone, Debug)]
struct TranscriptWord {
    text: String,
    start: f32,
    end: f32,
    stable_updates: u8,
    committed: bool,
}

#[derive(Default)]
pub struct TranscriptManager {
    words: Vec<TranscriptWord>,
}

const ROLLBACK_WORDS: usize = 3;
const WORD_TIME_TOLERANCE_SECS: f32 = 0.45;

impl TranscriptManager {
    /// Merge a window hypothesis into the session transcript using absolute word times.
    /// Words outside this audio window are retained; the sliding window never owns history.
    pub fn update(
        &mut self,
        hypotheses: &[WordHypothesis],
        window_start: f32,
        window_end: f32,
    ) -> Vec<String> {
        if hypotheses.is_empty() {
            return self.current_words();
        }

        let mut hypotheses = hypotheses.to_vec();
        hypotheses.sort_by(|a, b| a.start.total_cmp(&b.start));
        let committed_indices: Vec<usize> = self
            .words
            .iter()
            .enumerate()
            .filter_map(|(i, word)| word.committed.then_some(i))
            .collect();
        let rollback_start = committed_indices.len().saturating_sub(ROLLBACK_WORDS);
        let rollback: Vec<bool> = self
            .words
            .iter()
            .enumerate()
            .map(|(i, word)| word.committed && committed_indices[rollback_start..].contains(&i))
            .collect();

        let mut matched_words = vec![false; self.words.len()];
        let mut matched_hypotheses = vec![false; hypotheses.len()];

        // Locked words consume their matching hypothesis but can never be revised.
        for i in 0..self.words.len() {
            if !self.words[i].committed
                || rollback[i]
                || !in_window(&self.words[i], window_start, window_end)
            {
                continue;
            }
            if let Some(j) = find_candidate(&self.words[i], &hypotheses, &matched_hypotheses, true)
            {
                matched_words[i] = true;
                matched_hypotheses[j] = true;
            }
        }
        for i in 0..self.words.len() {
            if !self.words[i].committed
                || rollback[i]
                || matched_words[i]
                || !in_window(&self.words[i], window_start, window_end)
            {
                continue;
            }
            if let Some(j) = find_candidate(&self.words[i], &hypotheses, &matched_hypotheses, false)
            {
                matched_words[i] = true;
                matched_hypotheses[j] = true;
            }
        }

        // Prefer exact word matches before considering text revisions, so a
        // neighboring word cannot steal a repeated or unchanged token.
        for i in 0..self.words.len() {
            if (self.words[i].committed && !rollback[i])
                || !in_window(&self.words[i], window_start, window_end)
            {
                continue;
            }
            if let Some(j) = find_candidate(&self.words[i], &hypotheses, &matched_hypotheses, true)
            {
                apply_match(&mut self.words[i], &hypotheses[j]);
                matched_words[i] = true;
                matched_hypotheses[j] = true;
            }
        }

        // Match remaining nearby words as revisions. An unmatched provisional
        // or rollback word in the current window was retracted by this result.
        for i in 0..self.words.len() {
            if matched_words[i]
                || (self.words[i].committed && !rollback[i])
                || !in_window(&self.words[i], window_start, window_end)
            {
                continue;
            }
            if let Some(j) = find_candidate(&self.words[i], &hypotheses, &matched_hypotheses, false)
            {
                apply_match(&mut self.words[i], &hypotheses[j]);
                matched_words[i] = true;
                matched_hypotheses[j] = true;
            }
        }

        let mut next = Vec::with_capacity(self.words.len() + hypotheses.len());
        for (i, word) in self.words.drain(..).enumerate() {
            if matched_words[i]
                || !in_window(&word, window_start, window_end)
                || (word.committed && !rollback[i])
            {
                next.push(word);
            }
        }
        for (i, hypothesis) in hypotheses.into_iter().enumerate() {
            if !matched_hypotheses[i] {
                next.push(TranscriptWord {
                    text: hypothesis.text,
                    start: hypothesis.start,
                    end: hypothesis.end,
                    stable_updates: 1,
                    committed: false,
                });
            }
        }
        next.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.words = next;
        self.current_words()
    }

    pub fn current_words(&self) -> Vec<String> {
        self.words.iter().map(|word| word.text.clone()).collect()
    }

    pub fn final_text(&self) -> String {
        self.words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn in_window(word: &TranscriptWord, start: f32, end: f32) -> bool {
    word.end >= start && word.start <= end
}

fn find_candidate(
    existing: &TranscriptWord,
    hypotheses: &[WordHypothesis],
    consumed: &[bool],
    exact_only: bool,
) -> Option<usize> {
    hypotheses
        .iter()
        .enumerate()
        .filter(|(i, hypothesis)| {
            !consumed[*i]
                && (!exact_only || existing.text == hypothesis.text)
                && (midpoint(existing.start, existing.end)
                    - midpoint(hypothesis.start, hypothesis.end))
                .abs()
                    <= WORD_TIME_TOLERANCE_SECS
        })
        .min_by(|(_, a), (_, b)| {
            candidate_cost(existing, a).total_cmp(&candidate_cost(existing, b))
        })
        .map(|(i, _)| i)
}

fn apply_match(existing: &mut TranscriptWord, candidate: &WordHypothesis) {
    if existing.text == candidate.text {
        existing.stable_updates = existing.stable_updates.saturating_add(1);
    } else {
        existing.text.clone_from(&candidate.text);
        existing.stable_updates = 1;
        existing.committed = false;
    }
    existing.start = candidate.start;
    existing.end = candidate.end;
    if existing.stable_updates >= 3 {
        existing.committed = true;
    }
}

fn candidate_cost(existing: &TranscriptWord, candidate: &WordHypothesis) -> f32 {
    let text_penalty = if existing.text == candidate.text {
        0.0
    } else {
        0.25
    };
    text_penalty
        + (midpoint(existing.start, existing.end) - midpoint(candidate.start, candidate.end)).abs()
}

fn midpoint(start: f32, end: f32) -> f32 {
    (start + end) / 2.0
}

#[cfg(test)]
mod tests {
    use super::{TranscriptManager, WordHypothesis};

    fn word(text: &str, start: f32) -> WordHypothesis {
        WordHypothesis {
            text: text.to_owned(),
            start,
            end: start + 0.2,
        }
    }

    #[test]
    fn preserves_words_after_the_audio_window_moves_forward() {
        let mut manager = TranscriptManager::default();
        for _ in 0..3 {
            manager.update(&[word("one", 0.2)], 0.0, 4.0);
        }
        for _ in 0..3 {
            manager.update(&[word("two", 4.2)], 4.0, 8.0);
        }
        assert_eq!(
            manager.current_words(),
            vec!["one".to_owned(), "two".to_owned()]
        );
    }

    #[test]
    fn revises_a_recently_committed_word() {
        let mut manager = TranscriptManager::default();
        for _ in 0..3 {
            manager.update(&[word("their", 1.0)], 0.0, 4.0);
        }
        manager.update(&[word("there", 1.0)], 0.0, 4.0);
        assert_eq!(manager.current_words(), vec!["there".to_owned()]);
    }

    #[test]
    fn keeps_committed_prefix_outside_the_rollback_window_immutable() {
        let mut manager = TranscriptManager::default();
        let original = [
            word("first", 0.1),
            word("second", 0.5),
            word("third", 0.9),
            word("fourth", 1.3),
        ];
        for _ in 0..3 {
            manager.update(&original, 0.0, 4.0);
        }
        let revised = [
            word("changed", 0.1),
            word("second", 0.5),
            word("third", 0.9),
            word("fourth", 1.3),
        ];
        manager.update(&revised, 0.0, 4.0);
        assert_eq!(
            manager.current_words(),
            vec![
                "first".to_owned(),
                "second".to_owned(),
                "third".to_owned(),
                "fourth".to_owned(),
            ]
        );
    }

    #[test]
    fn keeps_punctuation_attached_to_words() {
        let mut manager = TranscriptManager::default();
        manager.update(&[word("Hello,", 0.1), word("world!", 0.5)], 0.0, 4.0);
        assert_eq!(manager.final_text(), "Hello, world!");
    }

    #[test]
    fn only_commits_after_three_matching_updates() {
        let mut manager = TranscriptManager::default();
        manager.update(&[word("maybe", 0.2)], 0.0, 4.0);
        manager.update(&[word("maybe", 0.2)], 0.0, 4.0);
        manager.update(&[word("certainly", 0.2)], 0.0, 4.0);
        assert_eq!(manager.current_words(), vec!["certainly".to_owned()]);
    }
}
