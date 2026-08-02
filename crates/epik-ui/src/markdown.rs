//! Assistant output rendered as markdown.
//!
//! Raw HTML in the source is **escaped rather than passed through**. The model's
//! output is not trusted markup: it routinely contains text it read from a repo,
//! a web page, or a tool result, and this renders inside a WebView that can reach
//! Tauri's command surface. Passing `<script>` through would make any string the
//! model ever quotes a way to call `send_user` or `respond_permission`. Escaping
//! costs nothing — nobody needs the assistant to emit a `<div>` — and it means a
//! tool result containing markup renders as the markup it is.
//!
//! Streaming-safe by construction: a partial document renders as whatever it is
//! so far, so an unterminated code fence or a half-written list looks unfinished
//! rather than breaking the page.

use pulldown_cmark::{Event, Options, Parser, html};

/// Render `source` to HTML, escaping any raw HTML it contains.
pub fn to_html(source: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let escaped = Parser::new_ext(source, options).map(|event| match event {
        // The three ways raw markup reaches the output. Turned back into text so
        // it is displayed rather than interpreted.
        Event::Html(raw) => Event::Text(raw),
        Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });

    let mut out = String::with_capacity(source.len() + source.len() / 4);
    html::push_html(&mut out, escaped);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ordinary_markdown() {
        let html = to_html("Some **bold** and `code`.");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<code>code</code>"), "{html}");
    }

    #[test]
    fn renders_fenced_code_blocks() {
        let html = to_html("```rust\nfn main() {}\n```");
        assert!(html.contains("<pre>"), "{html}");
        assert!(html.contains("language-rust"), "{html}");
    }

    #[test]
    fn renders_lists_and_tables() {
        assert!(to_html("- one\n- two").contains("<li>one</li>"));
        assert!(to_html("| a | b |\n| --- | --- |\n| 1 | 2 |").contains("<table>"));
    }

    /// The security property. A model that quotes a repository's README must not
    /// be able to execute anything by doing so.
    #[test]
    fn raw_html_is_escaped_not_executed() {
        let html = to_html("<script>alert('x')</script>");
        assert!(
            !html.contains("<script>"),
            "raw HTML must not survive: {html}"
        );
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn inline_html_is_escaped_too() {
        let html = to_html("text with <img src=x onerror=alert(1)> inline");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }

    #[test]
    fn an_unterminated_code_fence_still_renders() {
        // Exactly what a half-streamed reply looks like.
        let html = to_html("Here you go:\n\n```rust\nfn main() {");
        assert!(html.contains("<pre>"), "{html}");
        assert!(html.contains("fn main() {"), "{html}");
    }

    #[test]
    fn partial_emphasis_does_not_break_the_output() {
        // Mid-stream, `**bo` has no closing marker; it should be literal text.
        let html = to_html("some **bo");
        assert!(html.contains("**bo"), "{html}");
    }

    #[test]
    fn empty_input_renders_to_nothing() {
        assert_eq!(to_html(""), "");
    }
}
