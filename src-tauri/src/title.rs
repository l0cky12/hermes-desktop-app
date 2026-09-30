//! Retitle: the prompt that asks the model for a Session title, and reading the title back
//! out of its reply. Hermes has no endpoint for this, so the client asks the model itself.

use serde_json::Value;

/// Characters kept from each message, and from the whole transcript.
const LINE_CHARS: usize = 400;
const TRANSCRIPT_CHARS: usize = 6000;

/// The whole request: instructions around the transcript. `None` when there is nothing to title.
/// A long Session keeps its opening message and as many of the latest as fit, so the title
/// covers where it ended up, not only how it started.
pub fn prompt(messages: &[Value]) -> Option<String> {
    let lines: Vec<String> = messages.iter().filter_map(line).collect();
    let (first, rest) = lines.split_first()?;
    let mut used = first.chars().count();
    let mut kept: Vec<&str> = rest
        .iter()
        .rev()
        .take_while(|l| {
            used += l.chars().count();
            used <= TRANSCRIPT_CHARS
        })
        .map(String::as_str)
        .collect();
    if kept.len() < rest.len() {
        kept.push("…");
    }
    kept.push(first);
    kept.reverse();
    Some(format!(
        "Write a short title (3 to 7 words) for the conversation below. Reply with only the title, \
         without quotes or a full stop. Don't answer or continue the conversation, and don't use any tools.\n\n\
         <conversation>\n{}\n</conversation>\n\nReply with the title only.",
        kept.join("\n")
    ))
}

/// Hermes's own titler takes more words than this as the model answering instead of titling;
/// Hermes refuses titles longer than 100 characters.
const MAX_WORDS: usize = 12;
const MAX_CHARS: usize = 100;

/// The title in a reply: its last non-empty line (past any "Here's a title:"), without a
/// `Title:` label, quotes, markdown, or a full stop. `None` when that isn't a usable title.
pub fn clean(reply: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).filter(|l| !l.is_empty()).last()?;
    let line = match line.get(..6) {
        Some(label) if label.eq_ignore_ascii_case("title:") => &line[6..],
        _ => line,
    };
    let wrapping = |c: char| "\"'“”‘’*`_ ".contains(c);
    let title = line.trim_start_matches(wrapping).trim_end_matches(|c| wrapping(c) || ".,;:".contains(c));
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = title.split(' ').count();
    (!title.is_empty() && words <= MAX_WORDS && title.chars().count() <= MAX_CHARS).then_some(title)
}

/// `User: …` or `Assistant: …` with the text on one line; tool calls and results are left out.
fn line(message: &Value) -> Option<String> {
    let speaker = match message["role"].as_str()? {
        "user" => "User",
        "assistant" => "Assistant",
        _ => return None,
    };
    let text = match &message["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(" "),
        _ => return None,
    };
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(LINE_CHARS).collect();
    (!text.is_empty()).then(|| format!("{speaker}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_prompt_quotes_each_user_and_assistant_message_on_one_line() {
        let messages = [
            json!({ "role": "user", "content": "The printer on\n floor 2  jams" }),
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "terminal" } }] }),
            json!({ "role": "tool", "content": "lpstat output" }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "Clear the queue." }, { "type": "image_url" }] }),
        ];
        assert_eq!(
            prompt(&messages).unwrap(),
            "Write a short title (3 to 7 words) for the conversation below. Reply with only the title, \
             without quotes or a full stop. Don't answer or continue the conversation, and don't use any tools.\n\n\
             <conversation>\nUser: The printer on floor 2 jams\nAssistant: Clear the queue.\n</conversation>\n\n\
             Reply with the title only."
        );
        assert_eq!(prompt(&[json!({ "role": "assistant", "content": " " })]), None);
        assert_eq!(prompt(&[]), None);
    }

    #[test]
    fn a_long_session_keeps_its_opening_and_its_latest_messages() {
        let mut messages = vec![json!({ "role": "user", "content": "Plan the Chromebook refresh" })];
        messages.extend((1..=40).map(|n| json!({ "role": "assistant", "content": format!("m{n} {}", "x".repeat(1000)) })));
        let text = prompt(&messages).unwrap();
        assert!(text.contains("User: Plan the Chromebook refresh\n…\n"), "opening, then a gap marker");
        assert!(text.contains("Assistant: m40 "), "the latest message");
        assert!(!text.contains("m1 "), "the middle is dropped");
        assert!(!text.contains(&"x".repeat(401)), "each message is cut short");
        assert!(text.len() < 8000, "{} chars", text.len());
    }

    #[test]
    fn the_title_is_read_out_of_the_reply() {
        assert_eq!(clean("Printer queue fix").as_deref(), Some("Printer queue fix"));
        assert_eq!(clean("Title: \"Fix the printer queue\".").as_deref(), Some("Fix the printer queue"));
        assert_eq!(clean("**Printer  queue fix**\n").as_deref(), Some("Printer queue fix"));
        assert_eq!(clean("Here's a title:\n\n“Printer queue fix”\n").as_deref(), Some("Printer queue fix"), "the last line");
        assert_eq!(clean(" \n\"\"\n"), None);
        let answer = "I cleared the queue and restarted CUPS, so the printer on floor two should work now.";
        assert_eq!(clean(answer), None, "a reply that answers instead of titling");
        assert_eq!(clean(&"W".repeat(101)), None, "Hermes refuses titles over 100 characters");
    }
}
