/// Builds the system prompt for transcript chat.
///
/// When the transcript exceeds `char_budget` characters, the oldest portion is
/// dropped so the most recent speech is always included.
pub fn build_chat_system_prompt(transcript_text: &str, char_budget: usize) -> String {
    let (transcript, truncated) = tail_chars(transcript_text, char_budget);
    let truncation_note = if truncated {
        "[earlier transcript truncated]\n"
    } else {
        ""
    };

    format!(
        "You are a helpful meeting assistant. Answer the user's questions using ONLY the meeting transcript below. \
If the answer is not in the transcript, say so. The meeting may still be in progress, so the transcript may be incomplete.\n\n\
--- MEETING TRANSCRIPT ---\n{}{}\n--- END TRANSCRIPT ---",
        truncation_note, transcript
    )
}

/// Returns the trailing `max_chars` characters of `text` on a char boundary,
/// plus whether truncation occurred. Avoids byte slicing (UTF-8 safety).
fn tail_chars(text: &str, max_chars: usize) -> (&str, bool) {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return (text, false);
    }

    let skip = char_count - max_chars;
    let byte_idx = text
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(0);
    (&text[byte_idx..], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_chars_no_truncation_when_within_budget() {
        let (out, truncated) = tail_chars("hello", 10);
        assert_eq!(out, "hello");
        assert!(!truncated);
    }

    #[test]
    fn tail_chars_keeps_most_recent_text() {
        let (out, truncated) = tail_chars("abcdef", 3);
        assert_eq!(out, "def");
        assert!(truncated);
    }

    #[test]
    fn tail_chars_is_utf8_safe() {
        // Multi-byte characters must not be split mid-codepoint
        let text = "héllo wörld 日本語テキスト";
        let (out, truncated) = tail_chars(text, 5);
        assert_eq!(out.chars().count(), 5);
        assert!(truncated);
    }

    #[test]
    fn system_prompt_includes_truncation_note() {
        let long_text = "x".repeat(100);
        let prompt = build_chat_system_prompt(&long_text, 10);
        assert!(prompt.contains("[earlier transcript truncated]"));
    }
}
