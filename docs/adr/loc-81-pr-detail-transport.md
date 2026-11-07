# PR detail transport

## Context

The viewer needs a PR's files, diffs, comments and reviews, and must write comments and reviews
back. The daemon already shells out to `gh` for PR status and actions.

## Decisions

1. **REST first, through `gh api`.** Reads and writes use REST endpoints. GraphQL stays only where
   REST has no equivalent (review thread resolution). One transport keeps auth, errors and exit
   codes uniform with the other `aidp gh` verbs.
2. **A review is submitted in a single request.** The review event, body and inline comments go in
   one `POST /pulls/:n/reviews`. The pending-review flow (create, add comments, submit) can leave a
   half-built review on GitHub when a step fails.
3. **Drafts are local.** Pending comments live in `~/.aidp/gateway/pr-drafts.json` until the review
   is submitted. They survive a reload and a daemon restart, and nothing is visible on GitHub early.
4. **Viewed marks are keyed on the blob sha.** `~/.aidp/gateway/pr-viewed.json` stores the path and
   the file's blob sha. A new push changes the sha, so the mark resets exactly when the content
   changed, without comparing commits.

## Consequences

- A failed submit leaves the drafts in place, so the user can retry.
- Approving or requesting changes on your own PR fails at GitHub; the daemon maps it to `409 own-pr`.
- Viewed marks are local to one machine.
