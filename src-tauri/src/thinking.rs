//! Helpers for removing model reasoning accidentally returned as visible text.
//!
//! Providers use several incompatible wrappers for this data.  Keep the list
//! deliberately limited to explicit protocol markers so ordinary Markdown and
//! XML in a user's answer are not mistaken for hidden reasoning.

const ANGLE_TAGS: &[&str] = &[
    "think",
    "thinking",
    "analysis",
    "reasoning",
    "thought",
    "thoughts",
    "reflection",
];
const BRACKET_TAGS: &[&str] = &[
    "think",
    "thinking",
    "analysis",
    "reasoning",
    "thought",
    "thoughts",
    "reflection",
];

/// Removes explicit, provider-style reasoning blocks from a model response.
///
/// Supported forms are case-insensitive `<think>`, `<thinking>`, `<analysis>`,
/// `<reasoning>`, `<thought>` (including attributes and nested blocks), plus
/// `[think]`, `<|think|>`, and `<!-- think -->` variants.  An unclosed opening
/// marker removes the remainder, because it is almost always a truncated
/// reasoning stream rather than an answer.
pub(crate) fn strip_thinking_blocks(text: &str) -> String {
    let mut answer = text.to_string();

    for tag in ANGLE_TAGS {
        answer = strip_angle_blocks(&answer, tag);
    }
    for tag in BRACKET_TAGS {
        answer = strip_fixed_blocks(&answer, &format!("[{tag}]"), &format!("[/{tag}]"));
        answer = strip_fixed_blocks(&answer, &format!("<|{tag}|>"), &format!("<|/{tag}|>"));
    }
    for tag in BRACKET_TAGS {
        answer = strip_fixed_blocks(
            &answer,
            &format!("<!-- {tag} -->"),
            &format!("<!-- /{tag} -->"),
        );
    }

    answer.trim().to_string()
}

fn strip_angle_blocks(input: &str, tag: &str) -> String {
    let mut answer = input.to_string();
    loop {
        let lower = answer.to_ascii_lowercase();
        let Some((start, open_end)) = find_angle_open(&lower, tag, 0) else {
            break;
        };
        if lower[start..open_end].trim_end().ends_with("/>") {
            answer.replace_range(start..open_end, "");
            continue;
        }
        let Some(end) = find_matching_angle_close(&lower, tag, open_end) else {
            answer.truncate(start);
            break;
        };
        answer.replace_range(start..end, "");
    }
    answer
}

fn strip_fixed_blocks(input: &str, open: &str, close: &str) -> String {
    let mut answer = input.to_string();
    loop {
        let lower = answer.to_ascii_lowercase();
        let Some(start) = lower.find(open) else { break };
        let after_open = start + open.len();
        let Some(end_relative) = lower[after_open..].find(close) else {
            answer.truncate(start);
            break;
        };
        let end = after_open + end_relative + close.len();
        answer.replace_range(start..end, "");
    }
    answer
}

fn find_angle_open(lower: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let marker = format!("<{tag}");
    let mut offset = from;
    while let Some(relative) = lower[offset..].find(&marker) {
        let start = offset + relative;
        let after_name = start + marker.len();
        let next = lower.as_bytes().get(after_name).copied();
        if matches!(next, Some(b'>') | Some(b'/')) || next.is_some_and(|b| b.is_ascii_whitespace())
        {
            if let Some(relative_end) = lower[after_name..].find('>') {
                return Some((start, after_name + relative_end + 1));
            }
            return Some((start, lower.len()));
        }
        offset = after_name;
    }
    None
}

fn find_matching_angle_close(lower: &str, tag: &str, from: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut cursor = from;
    let close_marker = format!("</{tag}");

    while cursor < lower.len() {
        let next_open = find_angle_open(lower, tag, cursor);
        let next_close = find_angle_close(lower, &close_marker, cursor);
        match (next_open, next_close) {
            (_, None) => return None,
            (Some((open_start, open_end)), Some((close_start, _close_end)))
                if open_start < close_start =>
            {
                // A self-closing tag does not add a nesting level.
                if !lower[open_start..open_end].trim_end().ends_with("/>") {
                    depth += 1;
                }
                cursor = open_end;
            }
            (_, Some((_, close_end))) => {
                depth -= 1;
                if depth == 0 {
                    return Some(close_end);
                }
                cursor = close_end;
            }
        }
    }
    None
}

fn find_angle_close(lower: &str, marker: &str, from: usize) -> Option<(usize, usize)> {
    let mut offset = from;
    while let Some(relative) = lower[offset..].find(marker) {
        let start = offset + relative;
        let after_name = start + marker.len();
        let next = lower.as_bytes().get(after_name).copied();
        if matches!(next, Some(b'>')) || next.is_some_and(|b| b.is_ascii_whitespace()) {
            if let Some(relative_end) = lower[after_name..].find('>') {
                return Some((start, after_name + relative_end + 1));
            }
            return Some((start, lower.len()));
        }
        offset = after_name;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::strip_thinking_blocks;

    #[test]
    fn removes_common_thinking_wrappers() {
        for input in [
            "<think>reasoning</think>answer",
            "<THINKING class=\"hidden\">reasoning</THINKING>answer",
            "[analysis]reasoning[/analysis]answer",
            "<|reasoning|>reasoning<|/reasoning|>answer",
            "<!-- thought -->reasoning<!-- /thought -->answer",
            "<think/>answer",
        ] {
            assert_eq!(strip_thinking_blocks(input), "answer", "{input}");
        }
    }

    #[test]
    fn removes_nested_and_unclosed_thinking() {
        assert_eq!(
            strip_thinking_blocks("before<think>a<think>b</think>c</think>after"),
            "beforeafter"
        );
        assert_eq!(
            strip_thinking_blocks("answer\n<reasoning>truncated"),
            "answer"
        );
    }

    #[test]
    fn preserves_unrelated_xml() {
        assert_eq!(
            strip_thinking_blocks("Use <note>this</note> literally."),
            "Use <note>this</note> literally."
        );
    }
}
