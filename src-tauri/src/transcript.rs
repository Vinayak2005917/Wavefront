use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerEvent {
    Partial {
        text: String,
    },
    Final {
        text: String,
        segment: Option<u64>,
    },
    Done {
        text: String,
    },
    Error {
        message: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Default, Debug, Clone)]
pub struct StreamingTranscript {
    pub committed_segments: Vec<String>,
    pub provisional: String,
    pub last_segment: Option<u64>,
}

impl StreamingTranscript {
    pub fn partial(&mut self, text: String) {
        self.provisional = text;
    }

    pub fn finalize(&mut self, text: String, segment: Option<u64>) {
        let text = text.trim().to_owned();
        if !text.is_empty() {
            // Segment numbers are unique within a stream; equal text can still be a real repetition.
            if segment.is_none() || self.last_segment != segment {
                self.committed_segments.push(text);
            }
            self.last_segment = segment.or(self.last_segment);
        }
        self.provisional.clear();
    }

    pub fn assembled(&self) -> String {
        let mut parts = self.committed_segments.clone();
        if !self.provisional.trim().is_empty() {
            parts.push(self.provisional.trim().to_owned());
        }
        parts.join(" ")
    }

    /// Use the server's session transcript as a consistency check without duplicating finals.
    pub fn reconcile_done(&mut self, text: &str) -> bool {
        let normalized = text.trim();
        let changed = self.assembled() != normalized;
        if changed {
            self.committed_segments = if normalized.is_empty() {
                vec![]
            } else {
                vec![normalized.to_owned()]
            };
            self.provisional.clear();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerEvent, StreamingTranscript};

    #[test]
    fn partial_is_replaced_and_can_shorten_or_be_empty() {
        let mut t = StreamingTranscript::default();
        t.partial("hello world".into());
        assert_eq!(t.assembled(), "hello world");
        t.partial("hello".into());
        assert_eq!(t.assembled(), "hello");
        t.partial(String::new());
        assert_eq!(t.assembled(), "");
    }
    #[test]
    fn finals_and_done_reconcile_once() {
        let mut t = StreamingTranscript::default();
        t.partial("Hello world".into());
        t.finalize("Hello, world!".into(), Some(1));
        assert_eq!(t.assembled(), "Hello, world!");
        t.partial("Next phrase".into());
        t.finalize("Next phrase.".into(), Some(2));
        assert_eq!(t.assembled(), "Hello, world! Next phrase.");
        assert!(!t.reconcile_done("Hello, world! Next phrase."));
        assert!(t.reconcile_done("Hello, world! Next phrase. Extra."));
        assert_eq!(t.assembled(), "Hello, world! Next phrase. Extra.");
    }
    #[test]
    fn unicode_and_protocol_events_are_typed() {
        let mut t = StreamingTranscript::default();
        t.partial("Cafe\u{301} 🐈".into());
        assert!(t.assembled().contains("🐈"));
        assert!(matches!(
            serde_json::from_str::<ServerEvent>(r#"{"type":"error","message":"busy"}"#),
            Ok(ServerEvent::Error { .. })
        ));
        assert!(matches!(
            serde_json::from_str::<ServerEvent>(r#"{"type":"partial","text":"ok"}"#),
            Ok(ServerEvent::Partial { .. })
        ));
    }
}
