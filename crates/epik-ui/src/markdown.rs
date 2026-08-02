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
//! Escaping raw HTML is not on its own enough, because markdown has a second
//! way to name a URL that the browser will execute: a link or image
//! destination. `[click](javascript:…)` contains no markup at all, and
//! `![](data:text/html,…)` fetches a document the model wrote. So a
//! destination is rendered live only when its scheme is `http` or `https`;
//! anything else — another scheme, or no scheme at all — renders as the plain
//! text it is. See [`renders_live`].
//!
//! Streaming-safe by construction: a partial document renders as whatever it is
//! so far, so an unterminated code fence or a half-written list looks unfinished
//! rather than breaking the page.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};

/// Render `source` to HTML, escaping any raw HTML it contains and dropping any
/// link or image whose destination is not `http`/`https`.
pub fn to_html(source: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    // One entry per open link/image tag, recording whether that tag was
    // dropped. Links cannot nest but an image can sit inside a link, so the
    // matching close needs a stack rather than a flag.
    let mut dropped: Vec<bool> = Vec::new();

    let filtered = Parser::new_ext(source, options).filter_map(move |event| match event {
        // The three ways raw markup reaches the output. Turned back into text so
        // it is displayed rather than interpreted.
        Event::Html(raw) => Some(Event::Text(raw)),
        Event::InlineHtml(raw) => Some(Event::Text(raw)),

        // A destination the browser must not be handed. Dropping the tag and
        // its close — rather than the events between them — leaves the link
        // text or the image's alt text rendering as ordinary prose, which is
        // what the model wrote and all it is entitled to.
        Event::Start(Tag::Link { ref dest_url, .. } | Tag::Image { ref dest_url, .. }) => {
            let drop_it = !renders_live(dest_url);
            dropped.push(drop_it);
            (!drop_it).then_some(event)
        }
        Event::End(TagEnd::Link | TagEnd::Image) => {
            // `unwrap_or(false)` cannot happen on well-formed events; keeping
            // the close is the harmless answer if it ever did.
            let drop_it = dropped.pop().unwrap_or(false);
            (!drop_it).then_some(event)
        }

        other => Some(other),
    });

    let mut out = String::with_capacity(source.len() + source.len() / 4);
    html::push_html(&mut out, filtered);
    out
}

/// Whether `dest` may be rendered as a live link or image source.
///
/// An allowlist of `http` and `https`, not a denylist of `javascript`: the set
/// of schemes a WebView will do something dangerous with is open-ended
/// (`javascript:`, `data:`, `blob:`, `file:`, `tauri:`, whatever the platform
/// adds next), while the set the assistant has any business linking to is two
/// entries long.
///
/// A destination with no scheme is not live either. That covers the
/// scheme-relative `//host/path`, which silently inherits the page's scheme,
/// and ordinary relative paths — the app is a bundle of local assets, so a
/// relative link is a way to navigate the window away from the interface
/// rather than a link to anywhere.
fn renders_live(dest: &str) -> bool {
    // What the WebView will see, not what the markdown says. Per the URL
    // standard a browser strips leading and trailing C0 controls and spaces
    // from a URL and removes every tab, newline and carriage return inside it,
    // so `java\nscript:alert(1)` is a live `javascript:` href to it. Checking
    // the raw string would be checking a URL nobody navigates to.
    let normalized: String = dest
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    // No colon means no scheme, and the whole string is compared against
    // nothing: `[x](http)` is a relative path to a file called `http`.
    let Some((scheme, _rest)) = normalized.split_once(':') else {
        return false;
    };
    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
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

    // -----------------------------------------------------------------------
    // Link and image destinations. The second way a URL reaches the browser,
    // and the one escaping raw HTML does nothing about.
    // -----------------------------------------------------------------------

    /// The property, stated once: whatever else the output contains, the
    /// destination is not in an attribute the browser will act on.
    fn assert_not_live(markdown: &str, destination: &str) {
        let html = to_html(markdown);
        assert!(
            !html.contains("href=") && !html.contains("src="),
            "{markdown:?} produced a live destination: {html}"
        );
        assert!(
            !html.contains(destination),
            "{destination:?} survived into the output: {html}"
        );
    }

    #[test]
    fn a_javascript_link_is_not_a_link() {
        assert_not_live("[x](javascript:alert(1))", "javascript:");
    }

    #[test]
    fn a_javascript_link_keeps_its_text_as_prose() {
        // Dropping the tag must not drop what the model actually said.
        let html = to_html("Try [clicking here](javascript:alert(1)) now.");
        assert!(html.contains("clicking here"), "{html}");
    }

    #[test]
    fn a_data_url_image_is_not_an_image() {
        assert_not_live("![alt](data:text/html;base64,PHNjcmlwdD4=)", "data:");
    }

    #[test]
    fn a_data_url_link_is_not_a_link() {
        assert_not_live("[x](data:text/html,<script>alert(1)</script>)", "data:text");
    }

    /// `//host/path` has no scheme: it inherits the page's, which inside the
    /// bundle is the app's own. It looks absolute and is not.
    #[test]
    fn a_scheme_relative_destination_is_not_a_link() {
        assert_not_live("[x](//evil.example/x)", "//evil.example");
    }

    #[test]
    fn a_scheme_relative_image_is_not_an_image() {
        assert_not_live("![x](//evil.example/x.png)", "//evil.example");
    }

    /// Case is not a defence: `JaVaScRiPt:` is the same scheme to a browser.
    #[test]
    fn scheme_matching_ignores_case_and_surrounding_space() {
        assert_not_live("[x](  JaVaScRiPt:alert(1)  )", "avaScRiPt");
    }

    /// A browser drops tabs and newlines inside a URL before resolving it, so a
    /// scheme split across them is still that scheme.
    #[test]
    fn a_scheme_broken_up_by_control_characters_is_still_that_scheme() {
        // The destination is written with an entity so the markdown parser
        // hands us the control character, exactly as a hostile document would.
        let html = to_html("[x](java&#9;script:alert&#40;1&#41;)");
        assert!(!html.contains("href="), "{html}");
    }

    #[test]
    fn other_schemes_are_not_links_either() {
        for markdown in [
            "[x](file:///etc/passwd)",
            "[x](vbscript:msgbox(1))",
            "[x](blob:https://example.com/abc)",
            "[x](tauri://localhost/x)",
        ] {
            let html = to_html(markdown);
            assert!(!html.contains("href="), "{markdown:?} → {html}");
        }
    }

    /// The other half of the property: ordinary links still work. A renderer
    /// that dropped everything would pass every test above and be useless.
    #[test]
    fn ordinary_http_and_https_links_still_render() {
        let html = to_html("[docs](https://claude.com/claude-code)");
        assert!(
            html.contains(r#"<a href="https://claude.com/claude-code">docs</a>"#),
            "{html}"
        );
        assert!(to_html("[x](http://example.com)").contains(r#"href="http://example.com""#));
        assert!(to_html("[x](HTTPS://Example.com)").contains(r#"href="HTTPS://Example.com""#));
    }

    #[test]
    fn ordinary_https_images_still_render() {
        let html = to_html("![a cat](https://example.com/cat.png)");
        assert!(html.contains(r#"src="https://example.com/cat.png""#), "{html}");
        assert!(html.contains(r#"alt="a cat""#), "{html}");
    }

    #[test]
    fn autolinks_still_render() {
        let html = to_html("<https://example.com/x>");
        assert!(html.contains(r#"href="https://example.com/x""#), "{html}");
    }

    /// An image inside a link: two open tags at once, so the close of the inner
    /// one must not be matched against the outer one's verdict.
    #[test]
    fn a_blocked_image_inside_a_live_link_leaves_the_link_intact() {
        let html = to_html("[![alt](javascript:alert(1))](https://example.com)");
        assert!(html.contains(r#"<a href="https://example.com">"#), "{html}");
        assert!(html.contains("</a>"), "link was not closed: {html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("alt"), "alt text should remain as prose: {html}");
    }

    #[test]
    fn a_live_image_inside_a_blocked_link_still_renders() {
        let html = to_html("[![alt](https://example.com/c.png)](javascript:alert(1))");
        assert!(!html.contains("href="), "{html}");
        assert!(html.contains(r#"src="https://example.com/c.png""#), "{html}");
    }

    /// Reference-style links take the same path, so the destination cannot be
    /// smuggled in by defining it somewhere else in the document.
    #[test]
    fn reference_style_destinations_are_filtered_too() {
        let html = to_html("[x][ref]\n\n[ref]: javascript:alert(1)");
        assert!(!html.contains("href="), "{html}");
    }

    /// A half-arrived link is a plain-text prefix, not a link with a truncated
    /// destination — the streaming case, checked because this filter runs on
    /// every partial render.
    #[test]
    fn a_partial_link_mid_stream_renders_as_text() {
        let html = to_html("see [docs](https://exa");
        assert!(!html.contains("href="), "{html}");
        assert!(html.contains("[docs]"), "{html}");
    }
}
