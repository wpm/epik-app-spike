# Loop: autonomous spike execution

You are running the epik-app Rust spike (see the goal prompt). Work unassisted until
the goal resolves. This loop is your operating rhythm; LOG.md is your memory.

## On every session start (including after compaction or restart)

1. Read `LOG.md` in full. It is the authoritative state. Trust it over your memory
   of this conversation.
2. Read the goal prompt's milestones and kill criteria (it lives on disk at
   `.claude/commands/spike-goal.md`).
3. State (to the log, not to Bill) the current milestone, the next smallest step,
   and any live risk. Then work.

## The loop

1. **Pick the smallest next step** that advances the current milestone. Prefer steps
   that could falsify the approach over steps that decorate it — hit the risk first.
2. **Work in small commits.** Every commit message names the milestone. Push at least
   every checkpoint.
3. **Checkpoint every ~2 hours of work** (or at any milestone boundary): append to
   LOG.md — timestamp, what got done, what is next, open risks, estimated spend so
   far. Keep entries terse; the log is for resuming, not for narrative.
4. **When blocked**: timebox 45 minutes on the direct route. If still blocked, log
   the blockage precisely (error text, versions, what was tried), attempt ONE
   alternate route, timebox that too. If both fail, decide: is this a kill-criterion
   candidate or routable? Route around if routable; otherwise advance the kill
   decision explicitly in the log and act on it.
5. **Dead ends are deliverables.** Anything tried and abandoned gets two sentences
   in FINDINGS.md immediately, while the evidence is fresh — not reconstructed at
   the end.
6. **Keep FINDINGS.md alive**, not final-hour. After every milestone, update the
   verdict-in-progress and the evidence sections. The final write-up should be an
   edit, not a composition.

## Rules of autonomy

- Do not ask Bill anything unless you are hard-blocked on a credential you cannot
  obtain or a kill/budget condition has fired. Preferences you are unsure about:
  make the call, log the rationale, mark it `[decision made on own authority]` so
  review can find it — same convention as Epik headless builds.
- Never touch the Epik repository, any local clone of it, or Epik's issue tracker.
  The spike repo is the entire universe of this work.
- clippy + rustfmt clean before every push. CI in the spike repo if it earns its
  keep; do not gold-plate a spike.
- Log estimated API spend at every checkpoint. Hard stop at the budget cap from the
  goal prompt.

## Stop conditions (the ONLY ways this loop ends)

- **Goal complete**: M3 demo works end-to-end (M4 if time permits). Finalize
  FINDINGS.md, push everything, write the final summary.
- **Kill criterion fired**: finalize FINDINGS.md with the RED verdict and evidence,
  push, write the final summary.
- **Budget cap or time limit** (from the goal prompt): report milestone reached,
  verdict-so-far, and what remains.
- **Hard credential block**: log exactly what is needed and why, push all work,
  summarize state for Bill.

## The final summary (last message of the run)

One screen, no more: verdict + one-paragraph justification, link to FINDINGS.md,
milestone reached, total estimated spend, and the single most important thing Bill
should look at first.
