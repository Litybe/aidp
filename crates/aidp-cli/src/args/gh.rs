//! The `aidp gh` subcommand tree: GitHub verbs, run through the `gh` CLI.

use crate::models::enums::diff_side_arg::DiffSideArg;
use crate::models::enums::review_event_arg::ReviewEventArg;
use crate::models::BotsArg;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Op {
    /// gh pr create
    PrCreate {
        /// Title of the pull request.
        #[arg(long)]
        title: String,
        /// Body of the pull request.
        #[arg(long)]
        body: String,
        /// Base branch; the repository default when omitted.
        #[arg(long)]
        base: Option<String>,
        /// Open the pull request as a draft.
        #[arg(long)]
        draft: bool,
    },
    /// Mark a PR ready for review, or back to draft with --undo.
    /// Emits {changed, draft}.
    PrReady {
        /// Pull request number.
        pr: u64,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
        /// Convert the PR back to a draft.
        #[arg(long)]
        undo: bool,
    },
    /// Rebase a PR branch onto its base through GitHub. Emits {head_sha}.
    PrUpdateBranch {
        /// Pull request number.
        pr: u64,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Squash-merge a PR whose merge gate is open, pinned to a head sha.
    PrMerge {
        /// Pull request number.
        pr: u64,
        /// Head commit sha the merge is pinned to; a moved branch is refused.
        #[arg(long)]
        expected_head_sha: String,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
        /// Keep the head branch after the merge.
        #[arg(long)]
        keep_branch: bool,
        /// Report a conflict with the base as `{merged: false, conflict: true}` (exit 0)
        /// instead of failing.
        #[arg(long)]
        report_conflict: bool,
    },
    /// Migrate a repository's labels to the aidp vocabulary: create the missing
    /// labels, `aidp:need-fix` to `aidp:fixing`, `aidp:done` with an open PR to
    /// `aidp:awaiting-merge`. Dry run unless `--apply`. Emits the plan.
    LabelsMigrate {
        /// Repository as `owner/name`.
        #[arg(long)]
        repo: String,
        /// Write the changes on GitHub; without it nothing is written.
        #[arg(long)]
        apply: bool,
        /// Prefix of the state labels.
        #[arg(long, default_value = "aidp:")]
        prefix: String,
    },
    /// Reply on a review thread by GraphQL node id.
    ThreadReply {
        /// GraphQL node id of the review thread.
        thread_id: String,
        /// File holding the reply body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
    /// Ask the daemon to start an autonomous fix session on selected review
    /// threads and comment points. Emits the daemon's answer.
    PrFix {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// GraphQL node ids of the review threads to fix, comma separated.
        #[arg(long, required = true, value_delimiter = ',')]
        threads: Vec<String>,
        /// Ids of the comment points to fix, comma separated.
        #[arg(long, value_delimiter = ',')]
        points: Vec<String>,
        /// File holding an extra note for the session; `-` reads standard input.
        #[arg(long)]
        note_file: Option<String>,
        /// Model the session runs on.
        #[arg(long)]
        model: Option<String>,
        /// Effort level of the session.
        #[arg(long)]
        effort: Option<String>,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Resolve review threads; exits 6 when any thread failed.
    /// Emits one {id, ok, error?} per thread.
    ThreadResolve {
        /// GraphQL node ids of the review threads.
        #[arg(required = true)]
        thread_ids: Vec<String>,
        /// Reopen the threads instead of resolving them.
        #[arg(long)]
        unresolve: bool,
    },
    /// gh pr view --json (projection: number, url, title, state, mergeable)
    PrView {
        /// Pull request number or URL.
        pr: String,
    },
    /// gh pr diff (returns text)
    PrDiff {
        /// Pull request number or URL.
        pr: String,
    },
    /// Resolve the PR in play: the argument, else the current branch PR.
    /// Emits {number, url, title, state, head_ref, base_ref, head_sha, is_draft, repo} or {found:false}.
    PrResolve {
        /// Pull request number or URL; defaults to the current branch PR.
        pr: Option<String>,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Every check run on a PR, bucketed. Emits {conclusion, failing[], pending[], all[]}.
    PrChecks {
        /// Pull request number or URL; defaults to the current branch PR.
        pr: Option<String>,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Merge readiness of a PR: checks, review, threads and a gate verdict.
    /// Emits the status document; the exit code is 0 whatever the verdict.
    PrStatus {
        /// Pull request number; defaults to the current branch PR.
        pr: Option<u64>,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
    },
    /// The squad's repositories. Emits {repos, degraded}.
    SquadRepos,
    /// Open pull requests of the squad with their lanes and my review state.
    /// Emits {fetched_at, me, repos, prs, degraded}.
    SquadPrs {
        /// Restrict the listing to one repository, as `owner/name`.
        #[arg(long)]
        repo: Option<String>,
        /// Restrict the listing to one author login.
        #[arg(long)]
        author: Option<String>,
        /// How bot-authored pull requests are treated.
        #[arg(long, value_enum, default_value_t = BotsArg::Hide)]
        bots: BotsArg,
    },
    /// Unresolved review threads on a PR, with the GraphQL node IDs a reply
    /// needs. Emits {total, unresolved, threads[]} with startLine, comments[],
    /// lastBy, addressed and waitingOnReviewer per thread.
    PrReviewThreads {
        /// Pull request number or URL; defaults to the current branch PR.
        pr: Option<String>,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
        /// Include threads already marked resolved.
        #[arg(long)]
        all: bool,
    },
    /// Conversation comments and review bodies of a PR, split into points.
    /// Emits one {id, author, bot, kind, points[]} per comment.
    PrComments {
        /// Pull request number.
        pr: u64,
        /// Repository as `owner/name`; defaults to the current repository.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Everything a PR page shows: status, reviews, commits, threads and comments.
    PrDetail {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
    },
    /// One page of the files a PR changes, with their patches.
    /// Emits {page, per_page, files[]}.
    PrFiles {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// Page to read, starting at 1.
        #[arg(long, default_value = "1")]
        page: u32,
        /// Files per page, capped by GitHub.
        #[arg(long, default_value = "30")]
        per_page: u32,
    },
    /// The unified diff of one file of a PR (returns text).
    PrFileDiff {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// Path of the file inside the repository.
        path: String,
    },
    /// Post a conversation comment on a PR. Emits {id, url}.
    CommentCreate {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// File holding the comment body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
    /// Post a conversation comment on a PR. Emits {id, url}.
    PrComment {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// File holding the comment body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
    /// Replace the text of a conversation comment. Emits {id, url}.
    CommentEdit {
        /// Repository as `owner/name`.
        repo: String,
        /// Id of the comment.
        id: u64,
        /// File holding the new body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
    /// Delete a conversation comment.
    CommentDelete {
        /// Repository as `owner/name`.
        repo: String,
        /// Id of the comment.
        id: u64,
    },
    /// Post one inline comment on the PR head commit, outside any review.
    /// Emits {id, url}.
    ReviewCommentCreate {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// File the comment is anchored to.
        #[arg(long)]
        path: String,
        /// Line of the diff the comment is anchored to.
        #[arg(long)]
        line: u32,
        /// Side of the diff the line belongs to.
        #[arg(long, value_enum, default_value_t = DiffSideArg::Right)]
        side: DiffSideArg,
        /// First line of a multi-line comment.
        #[arg(long)]
        start_line: Option<u32>,
        /// Side of the first line of a multi-line comment.
        #[arg(long, value_enum)]
        start_side: Option<DiffSideArg>,
        /// Head commit sha; resolved from the PR when omitted.
        #[arg(long)]
        head_sha: Option<String>,
        /// File holding the comment body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
    /// Replace the text of an inline review comment. Emits {id, url}.
    ReviewCommentEdit {
        /// Repository as `owner/name`.
        repo: String,
        /// Id of the comment.
        id: u64,
        /// File holding the new body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
    /// Delete an inline review comment.
    ReviewCommentDelete {
        /// Repository as `owner/name`.
        repo: String,
        /// Id of the comment.
        id: u64,
    },
    /// Submit a review and all its inline comments in one request.
    /// Emits {id, state, url, comment_count}.
    ReviewSubmit {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// Verdict of the review.
        #[arg(long, value_enum)]
        event: ReviewEventArg,
        /// File holding the review summary; `-` reads standard input.
        #[arg(long)]
        body_file: String,
        /// JSON file holding the inline comments, an array of drafts.
        #[arg(long)]
        comments_file: String,
        /// Head commit sha; resolved from the PR when omitted.
        #[arg(long)]
        head_sha: Option<String>,
    },
    /// Post review findings as inline comments, idempotent per run and pass.
    /// Emits {posted, reviewId, threads[]}.
    ReviewPost {
        /// Repository as `owner/name`.
        repo: String,
        /// Pull request number.
        number: u64,
        /// JSON file holding the findings to post.
        #[arg(long)]
        findings: PathBuf,
        /// Review pass number carried in the finding markers.
        #[arg(long)]
        pass: u32,
        /// Run id carried in the finding markers.
        #[arg(long)]
        run: String,
    },
    /// Reply on a review thread, then reopen it.
    /// Emits {threadId, replied, unresolved}.
    ThreadReopen {
        /// GraphQL node id of the review thread.
        thread_id: String,
        /// File holding the reply body; `-` reads standard input.
        #[arg(long)]
        body_file: String,
    },
}
