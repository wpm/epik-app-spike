# Epik build monitoring loop

Default `/loop` body for Epik projects: watch the active headless feature
build and interrupt only when a human is needed. (The `epik:init-repository`
skill copies this file to a project's `.claude/loop.md` so a bare `/loop`
defaults to build monitoring.)

Each iteration:

1. Identify the build being watched. If the conversation already names a
   feature issue, use it. Otherwise find the most recent `Epik Build` run:
   `run_list` for this repo with workflow `epik-build.yml`, and read the
   feature issue number from the run name.
   If there is no Epik Build run at all, say so once and stop looping.
2. Gather status:
   - `feature_status` for the feature issue: sub-issue states, linked PRs,
     CI conclusions, dependency positions.
   - `run_list` for workflow `epik-build.yml` (the build session itself) and
     for recent runs on the feature and issue branches (their CI).
3. Print one compact status table: each sub-issue with its state, PR, and CI
   result, plus the build run's status. No other commentary when nothing
   changed.
4. Interrupt the user only on needs-me events:
   - The Epik Build run concluded with failure, or was cancelled.
   - A sub-issue PR's CI has failed and stayed failed across iterations.
   - The build posted a comment on the feature issue asking for a human
     decision (check new comments with `issue_get` / `gh_raw` when the build
     run has stalled).
   - The feature completed: all sub-issues closed, their PRs merged into the
     feature branch, and the build's review pull request into the base branch
     open. Announce completion and link that pull request — it is where to
     review, and merging it is the user's call.
5. Stop looping when the feature completes or the build run ends in failure;
   otherwise continue on a relaxed cadence (every few minutes is plenty —
   headless builds take a while).
