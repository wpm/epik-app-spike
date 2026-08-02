//! The status bar: what the session is doing, what it costs, and how it ended.
//!
//! One line along the bottom of the window. Everything here is derived from the
//! event stream, so it cannot disagree with the transcript above it — the state
//! dot and the transcript are two views of the same `Transcript` value.

use leptos::prelude::*;

use crate::session::SessionClient;
use crate::transcript::Status;

/// Cost with enough precision to be useful.
///
/// A turn costs cents, and rounding to two decimal places would show most
/// sessions as `$0.00` — which reads as "free" rather than "not much". Four
/// places is the granularity the CLI itself reports.
fn format_cost(total: f64) -> String {
    format!("${total:.4}")
}

#[component]
pub fn StatusBar(client: SessionClient) -> impl IntoView {
    let transcript = client.transcript;

    let status = Memo::new(move |_| transcript.with(|t| t.status));
    let model = Memo::new(move |_| transcript.with(|t| t.model.clone()));
    let cli_version = Memo::new(move |_| transcript.with(|t| t.cli_version.clone()));
    let cost = Memo::new(move |_| transcript.with(|t| t.cost_total_usd));
    let turns = Memo::new(move |_| transcript.with(|t| t.turns));
    let exit_code = Memo::new(move |_| transcript.with(|t| t.exit_code));
    let error_subtype = Memo::new(move |_| transcript.with(|t| t.error_subtype.clone()));

    view! {
        <div class="shrink-0 flex items-center gap-3 px-4 py-1.5 border-t border-line
                    bg-bar text-[11px] font-mono text-fg-muted">
            <StateDot status=status.into() />

            <span class=move || match status.get() {
                Status::Closed => "text-fg-secondary",
                Status::Streaming => "text-accent",
                _ => "text-fg-muted",
            }>
                {move || status.get().label()}
            </span>

            // The exit code sits next to the state because "closed" without it
            // does not answer the only question a closed session raises: whether
            // it ended or died.
            {move || (status.get() == Status::Closed).then(|| {
                let code = exit_code.get();
                let clean = code == Some(0);
                view! {
                    <span class=if clean { "text-fg-muted" } else { "text-error" }>
                        {match code {
                            Some(code) => format!("exit {code}"),
                            // No code means a signal killed it — which is what
                            // happens when the engine is killed out of band.
                            None => "killed (no exit code)".to_owned(),
                        }}
                    </span>
                }
            })}

            // An error result subtype is flagged rather than folded into the
            // state: a session can be idle and healthy after a turn that failed,
            // and those are different facts.
            {move || error_subtype.get().map(|subtype| view! {
                <span
                    class="px-1.5 py-0.5 rounded bg-error-muted text-error"
                    title="the last turn ended in an error result"
                >
                    {subtype}
                </span>
            })}

            <span class="ml-auto flex items-center gap-3">
                {move || {
                    let model = model.get();
                    (!model.is_empty()).then(|| view! {
                        <span class="text-fg-secondary selectable">{model}</span>
                    })
                }}
                {move || {
                    let version = cli_version.get();
                    (!version.is_empty()).then(|| view! {
                        <span class="selectable" title="Claude Code version">
                            {format!("cli {version}")}
                        </span>
                    })
                }}
                <span title="turns completed in this session">
                    {move || {
                        let turns = turns.get();
                        format!("{turns} turn{}", if turns == 1 { "" } else { "s" })
                    }}
                </span>
                <span
                    class="text-fg-secondary selectable"
                    title="cumulative cost: the sum of every turn's total_cost_usd"
                >
                    {move || format_cost(cost.get())}
                </span>
            </span>
        </div>
    }
}

#[component]
fn StateDot(status: Signal<Status>) -> impl IntoView {
    view! {
        <span
            class=move || {
                let tone = match status.get() {
                    Status::Offline => "bg-fg-faint",
                    Status::Starting => "bg-warning animate-pulse",
                    Status::Idle => "bg-success",
                    Status::Streaming => "bg-accent animate-pulse",
                    Status::Closed => "bg-fg-muted",
                };
                format!("w-1.5 h-1.5 rounded-full shrink-0 {tone}")
            }
            title=move || status.get().label()
        ></span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_is_shown_to_four_places_so_cents_are_visible() {
        // Two decimals would render a real session as "$0.00", which reads as
        // free rather than as not much.
        assert_eq!(format_cost(0.0), "$0.0000");
        assert_eq!(format_cost(0.0184), "$0.0184");
        assert_eq!(format_cost(0.0184 + 0.0231), "$0.0415");
        assert_eq!(format_cost(1.5), "$1.5000");
    }
}
