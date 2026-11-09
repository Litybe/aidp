use super::*;
use aidp_testkit::TempWorkspace;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

const REPOSITORY_REPLY: &str = include_str!("fixtures/branch_publish/repository_reply.json");
const REF_REPLY: &str = include_str!("fixtures/branch_publish/ref_reply.json");
const COMMIT_REPLY: &str = include_str!("fixtures/branch_publish/commit_reply.json");
const ERRORS_REPLY: &str = include_str!("fixtures/branch_publish/errors_reply.json");
const CREATE_COMMIT_REQUEST: &str =
    include_str!("fixtures/branch_publish/create_commit_request.json");
const CREATE_REF_REQUEST: &str = include_str!("fixtures/branch_publish/create_ref_request.json");

const TAG: &str = "branch-publish";
const BRANCH: &str = "feat/PIXEL-1";
const OTHER_BRANCH: &str = "feat/PIXEL-2";
const DEFAULT_BRANCH: &str = "master";
const ORIGIN_HTTPS: &str = "https://github.com/acme/widgets.git";
const ORIGIN_WITH_CREDENTIAL: &str = "https://x-access-token:secret@github.com/acme/widgets.git";
const ORIGIN_SCP: &str = "git@github.com:acme/widgets.git";
const ORIGIN_SSH: &str = "ssh://git@github.com/acme/widgets";
const ORIGIN_WITHOUT_OWNER: &str = "widgets";
const OWNER: &str = "acme";
const NAME: &str = "widgets";
const BASE_OID: &str = "base123";
const REMOTE_OID: &str = "remote789";
const SIGNED_OID: &str = "5167e3d0signed";
const REPOSITORY_ID: &str = "R_kgDOwidgets";
const CHANGED_PATH: &str = "src/widget.rs";
const ADDED_PATH: &str = "docs/widget.md";
const DELETED_PATH: &str = "old.txt";
const SPACED_PATH: &str = "docs/a file.md";
const SUBMODULE_PATH: &str = "vendor/sub";
const CHANGED_CONTENT: &str = "hello";
const ADDED_CONTENT: &str = "doc";
const MODIFIED_STATUS: &str = "M";
const ADDED_STATUS: &str = "A";
const DELETED_STATUS_TEXT: &str = "D";
const HEADLINE: &str = "feat(PIXEL-1): add the widget";
const BODY: &str = "Explain why the widget exists.";
const OVERSIZED: &str = "999999999";
const GIT_ERROR_EXIT: i32 = 128;

/// One scripted reply of a tool: exit code and stdout.
#[derive(Clone)]
struct Answer {
    code: i32,
    stdout: Vec<u8>,
}

impl Answer {
    fn ok(stdout: &str) -> Self {
        Answer {
            code: 0,
            stdout: stdout.as_bytes().to_vec(),
        }
    }

    fn failed(code: i32) -> Self {
        Answer {
            code,
            stdout: Vec::new(),
        }
    }
}

/// Answers git lines from a table and `gh` calls in order; records lines and GraphQL bodies.
#[derive(Default)]
struct FakeTools {
    git: Mutex<HashMap<String, Answer>>,
    graphql: Mutex<VecDeque<Answer>>,
    calls: Mutex<Vec<String>>,
    bodies: Mutex<Vec<Value>>,
}

impl FakeTools {
    fn with_git(self, args: &[&str], answer: Answer) -> Self {
        self.git.lock().unwrap().insert(git_line(args), answer);
        self
    }

    fn with_graphql(self, reply: &str) -> Self {
        self.graphql.lock().unwrap().push_back(Answer::ok(reply));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().clone()
    }

    fn called(&self, args: &[&str]) -> bool {
        self.calls().contains(&git_line(args))
    }
}

fn exit_status(code: i32) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
}

impl ProcessRunner for FakeTools {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        let line = std::iter::once(spec.program.clone())
            .chain(spec.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        self.calls.lock().unwrap().push(line.clone());
        let answer = if spec.program == GH_PROGRAM {
            let body_path = spec.args.last().expect("gh gets a body file");
            let body = std::fs::read_to_string(body_path).expect("the body file exists");
            self.bodies
                .lock()
                .unwrap()
                .push(serde_json::from_str(&body).unwrap());
            self.graphql.lock().unwrap().pop_front()
        } else {
            self.git.lock().unwrap().get(&line).cloned()
        };
        let answer = answer.unwrap_or_else(|| Answer::failed(GIT_ERROR_EXIT));
        Ok(Output {
            status: exit_status(answer.code),
            stdout: answer.stdout,
            stderr: Vec::new(),
        })
    }

    fn spawn(
        &self,
        _spec: &ProcessSpec,
    ) -> std::io::Result<Box<dyn aidp_core::process_runner::SpawnedChild>> {
        Err(std::io::Error::other("publishing never spawns"))
    }
}

fn git_line(args: &[&str]) -> String {
    std::iter::once(GIT_PROGRAM)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

fn object(path: &str) -> String {
    format!("{HEAD}{OBJECT_SEPARATOR}{path}")
}

fn remote_ref(branch: &str) -> String {
    format!("{REMOTE_REF_PREFIX}{branch}")
}

fn diff_args(base: &str) -> Vec<&str> {
    vec![
        CONFIG_FLAG,
        LITERAL_PATHS,
        DIFF,
        NO_RENAMES_FLAG,
        NAME_STATUS_FLAG,
        NUL_FLAG,
        base,
        HEAD,
    ]
}

fn listing(records: &[(&str, &str)]) -> String {
    records
        .iter()
        .flat_map(|(status, path)| [*status, *path])
        .map(|token| format!("{token}\0"))
        .collect()
}

/// A worktree on `branch`, its origin on GitHub with `master` as default branch.
fn on_branch(branch: &str) -> FakeTools {
    FakeTools::default()
        .with_git(
            &[SYMBOLIC_REF, QUIET_FLAG, SHORT_FLAG, HEAD],
            Answer::ok(&format!("{branch}\n")),
        )
        .with_git(
            &[CONFIG, GET_FLAG, ORIGIN_URL_KEY],
            Answer::ok(ORIGIN_HTTPS),
        )
        .with_git(&[FETCH, ORIGIN, PRUNE_FLAG], Answer::ok(""))
        .with_graphql(REPOSITORY_REPLY)
}

/// A first publish: the branch is absent on GitHub and `HEAD` changes, adds and deletes files.
fn first_publish() -> FakeTools {
    let changed = object(CHANGED_PATH);
    let added = object(ADDED_PATH);
    let submodule = object(SUBMODULE_PATH);
    let default_ref = remote_ref(DEFAULT_BRANCH);
    on_branch(BRANCH)
        .with_git(&[MERGE_BASE, HEAD, &default_ref], Answer::ok(BASE_OID))
        .with_git(
            &diff_args(BASE_OID),
            Answer::ok(&listing(&[
                (MODIFIED_STATUS, CHANGED_PATH),
                (ADDED_STATUS, ADDED_PATH),
                (DELETED_STATUS_TEXT, DELETED_PATH),
                (MODIFIED_STATUS, SUBMODULE_PATH),
            ])),
        )
        .with_git(
            &[CAT_FILE, SIZE_FLAG, &changed],
            Answer::ok(&CHANGED_CONTENT.len().to_string()),
        )
        .with_git(&[CAT_FILE, BLOB, &changed], Answer::ok(CHANGED_CONTENT))
        .with_git(
            &[CAT_FILE, SIZE_FLAG, &added],
            Answer::ok(&ADDED_CONTENT.len().to_string()),
        )
        .with_git(&[CAT_FILE, BLOB, &added], Answer::ok(ADDED_CONTENT))
        .with_git(
            &[CAT_FILE, SIZE_FLAG, &submodule],
            Answer::failed(GIT_ERROR_EXIT),
        )
        .with_git(
            &[LOG, LAST_COMMIT_FLAG, MESSAGE_FORMAT, HEAD],
            Answer::ok(&format!("{HEADLINE}\n\n{BODY}\n")),
        )
        .with_graphql(REF_REPLY)
        .with_graphql(COMMIT_REPLY)
        .with_git(&[FETCH, ORIGIN, BRANCH], Answer::ok(""))
        .with_git(&[RESET, SOFT_FLAG, SIGNED_OID], Answer::ok(""))
}

/// A publish onto a branch GitHub has: `REMOTE_OID` is its head and an ancestor of `HEAD`.
fn existing_branch_publish() -> FakeTools {
    let branch_ref = remote_ref(BRANCH);
    let changed = object(CHANGED_PATH);
    on_branch(BRANCH)
        .with_git(
            &[REV_PARSE, VERIFY_FLAG, QUIET_FLAG, &branch_ref],
            Answer::ok(REMOTE_OID),
        )
        .with_git(
            &diff_args(REMOTE_OID),
            Answer::ok(&listing(&[(MODIFIED_STATUS, CHANGED_PATH)])),
        )
        .with_git(
            &[CAT_FILE, SIZE_FLAG, &changed],
            Answer::ok(&CHANGED_CONTENT.len().to_string()),
        )
        .with_git(&[CAT_FILE, BLOB, &changed], Answer::ok(CHANGED_CONTENT))
        .with_git(
            &[MERGE_BASE, IS_ANCESTOR_FLAG, REMOTE_OID, HEAD],
            Answer::ok(""),
        )
        .with_git(
            &[LOG, LAST_COMMIT_FLAG, MESSAGE_FORMAT, HEAD],
            Answer::ok(&format!(
                "{HEADLINE}

{BODY}
"
            )),
        )
        .with_graphql(COMMIT_REPLY)
        .with_git(&[FETCH, ORIGIN, BRANCH], Answer::ok(""))
        .with_git(&[RESET, SOFT_FLAG, SIGNED_OID], Answer::ok(""))
}

fn is_nothing_to_publish(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<TypedExit>()
        .is_some_and(|typed| typed.status == ExitStatus::NegativeVerdict)
}

fn publish_with(tools: &FakeTools, expected: Option<&str>) -> anyhow::Result<FlowPushReport> {
    let workspace = TempWorkspace::new(TAG);
    let context = PublishContext {
        runner: tools,
        workspace: workspace.path(),
        scratch: workspace.path(),
    };
    publish(&context, expected, None)
}

fn is_conflict(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<IntegrationError>(),
        Some(IntegrationError::Conflict(_))
    )
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// HTTPS, credential-bearing, scp-like and `ssh://` origins all name `acme/widgets`.
#[test]
fn every_github_origin_form_names_the_owner_and_the_repository() {
    let expected = GithubRepository {
        owner: OWNER.to_string(),
        name: NAME.to_string(),
    };
    for url in [ORIGIN_HTTPS, ORIGIN_WITH_CREDENTIAL, ORIGIN_SCP, ORIGIN_SSH] {
        assert_eq!(parse_owner_repo(url), Some(expected.clone()), "{url}");
    }
    assert_eq!(parse_owner_repo(ORIGIN_WITHOUT_OWNER), None);
    assert_eq!(expected.name_with_owner(), format!("{OWNER}/{NAME}"));
}

/// A NUL-separated listing reads as status and path pairs, paths with spaces kept whole.
#[test]
fn name_status_listing_reads_as_status_and_path_pairs() {
    let text = listing(&[
        (MODIFIED_STATUS, SPACED_PATH),
        (DELETED_STATUS_TEXT, DELETED_PATH),
    ]);
    assert_eq!(
        name_status_entries(text.as_bytes()),
        vec![
            (MODIFIED_STATUS.to_string(), SPACED_PATH.to_string()),
            (DELETED_STATUS_TEXT.to_string(), DELETED_PATH.to_string()),
        ]
    );
}

/// The last local message gives the headline and the body; an empty one names the branch.
#[test]
fn the_commit_message_is_the_last_local_message() {
    let message = commit_message(&format!("{HEADLINE}\n\n{BODY}\n"), BRANCH);
    assert_eq!(message.headline, HEADLINE);
    assert_eq!(message.body.as_deref(), Some(BODY));
    let single = commit_message(HEADLINE, BRANCH);
    assert_eq!(single.body, None);
    let empty = commit_message("  \n", BRANCH);
    assert_eq!(empty.headline, format!("{DEFAULT_MESSAGE_PREFIX}{BRANCH}"));
}

/// The ref request roots the new branch at the commit the work started from.
#[test]
fn the_create_ref_request_matches_the_graphql_contract() {
    let request = create_ref_request(REPOSITORY_ID, BRANCH, BASE_OID);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json(CREATE_REF_REQUEST)
    );
}

/// A first publish roots the branch, sends whole base64 files and lands the worktree on the signed commit.
#[test]
fn a_first_publish_creates_the_branch_then_one_signed_commit() {
    let tools = first_publish();

    let report = publish_with(&tools, Some(BRANCH)).unwrap();

    assert_eq!(
        report,
        FlowPushReport {
            branch: BRANCH.to_string(),
            sha: SIGNED_OID.to_string(),
            changed: true,
        }
    );
    let bodies = tools.bodies();
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[1], json(CREATE_REF_REQUEST));
    assert_eq!(bodies[2], json(CREATE_COMMIT_REQUEST));
    let calls = tools.calls();
    let tail: Vec<String> = calls[calls.len() - 2..].to_vec();
    assert_eq!(
        tail,
        vec![
            git_line(&[FETCH, ORIGIN, BRANCH]),
            git_line(&[RESET, SOFT_FLAG, SIGNED_OID]),
        ]
    );
    assert!(!tools.called(&[CAT_FILE, BLOB, &object(SUBMODULE_PATH)]));
}

/// A publish onto an existing remote branch skips createRef, expects the remote head and resets onto the returned sha.
#[test]
fn a_publish_onto_an_existing_branch_commits_on_the_remote_head() {
    let tools = existing_branch_publish();

    let report = publish_with(&tools, Some(BRANCH)).unwrap();

    assert_eq!(
        report,
        FlowPushReport {
            branch: BRANCH.to_string(),
            sha: SIGNED_OID.to_string(),
            changed: true,
        }
    );
    assert!(tools.called(&[MERGE_BASE, IS_ANCESTOR_FLAG, REMOTE_OID, HEAD]));
    let bodies = tools.bodies();
    assert_eq!(bodies.len(), 2);
    let input = &bodies[1]["variables"]["input"];
    assert_eq!(input["expectedHeadOid"], REMOTE_OID);
    assert!(bodies[1]["query"]
        .as_str()
        .unwrap()
        .contains("createCommitOnBranch"));
    assert!(tools.called(&[RESET, SOFT_FLAG, SIGNED_OID]));
    assert!(!tools.called(&[RESET, SOFT_FLAG, &remote_ref(BRANCH)]));
}

/// A push landing between the mutation and the fetch is not absorbed: the worktree lands on the returned sha.
#[test]
fn the_worktree_lands_on_the_sha_github_returned() {
    let tools = existing_branch_publish();

    let report = publish_with(&tools, Some(BRANCH)).unwrap();

    assert_eq!(report.sha, SIGNED_OID);
    let calls = tools.calls();
    assert_eq!(
        calls.last().unwrap(),
        &git_line(&[RESET, SOFT_FLAG, SIGNED_OID])
    );
}

/// A new branch with no change against the default branch is a negative verdict, not a retryable failure.
#[test]
fn a_new_branch_with_nothing_to_publish_is_a_negative_verdict() {
    let tools = on_branch(BRANCH)
        .with_git(
            &[MERGE_BASE, HEAD, &remote_ref(DEFAULT_BRANCH)],
            Answer::ok(BASE_OID),
        )
        .with_git(&diff_args(BASE_OID), Answer::ok(""));

    let error = publish_with(&tools, Some(BRANCH)).unwrap_err();

    assert!(is_nothing_to_publish(&error), "{error}");
    assert!(error.to_string().contains(NOTHING_TO_PUBLISH));
    assert!(error.to_string().contains(DEFAULT_BRANCH));
    assert_eq!(tools.bodies().len(), 1);
}

/// The default branch is never published: refused before any fetch or write.
#[test]
fn the_default_branch_is_refused() {
    let tools = on_branch(DEFAULT_BRANCH);

    let error = publish_with(&tools, None).unwrap_err();

    assert!(is_conflict(&error), "{error}");
    assert!(error.to_string().contains(DEFAULT_BRANCH_REFUSED));
    assert!(!tools.called(&[FETCH, ORIGIN, PRUNE_FLAG]));
    assert_eq!(tools.bodies().len(), 1);
}

/// A worktree on another branch than the flow's is refused before anything is read.
#[test]
fn another_branch_than_the_expected_one_is_refused() {
    let tools = on_branch(OTHER_BRANCH);

    let error = publish_with(&tools, Some(BRANCH)).unwrap_err();

    assert!(is_conflict(&error), "{error}");
    assert!(tools.bodies().is_empty());
}

/// A detached HEAD names no branch to publish.
#[test]
fn a_detached_head_is_refused() {
    let tools = FakeTools::default();

    let error = publish_with(&tools, None).unwrap_err();

    assert!(is_conflict(&error), "{error}");
    assert!(error.to_string().contains(DETACHED_HEAD));
}

/// Nothing to publish on a branch GitHub has: the worktree lands on the remote head, unchanged.
#[test]
fn nothing_to_publish_repeats_without_creating_a_commit() {
    let branch_ref = remote_ref(BRANCH);
    let tools = on_branch(BRANCH)
        .with_graphql(REPOSITORY_REPLY)
        .with_git(
            &[REV_PARSE, VERIFY_FLAG, QUIET_FLAG, &branch_ref],
            Answer::ok(REMOTE_OID),
        )
        .with_git(&diff_args(REMOTE_OID), Answer::ok(""))
        .with_git(&[RESET, SOFT_FLAG, &branch_ref], Answer::ok(""));

    let report = publish_with(&tools, Some(BRANCH)).unwrap();

    assert!(!report.changed);
    assert_eq!(report.sha, REMOTE_OID);
    assert!(tools.called(&[RESET, SOFT_FLAG, &branch_ref]));
    let repeated = publish_with(&tools, Some(BRANCH)).unwrap();
    assert_eq!(repeated, report);
    assert_eq!(tools.bodies().len(), 2);
    assert!(!tools.called(&[LOG, LAST_COMMIT_FLAG, MESSAGE_FORMAT, HEAD]));
    assert!(!tools.called(&[FETCH, ORIGIN, BRANCH]));
}

/// Commits pushed by someone else since the worktree last synced are never overwritten.
#[test]
fn a_remote_branch_ahead_of_the_worktree_is_refused() {
    let branch_ref = remote_ref(BRANCH);
    let changed = object(CHANGED_PATH);
    let tools = on_branch(BRANCH)
        .with_git(
            &[REV_PARSE, VERIFY_FLAG, QUIET_FLAG, &branch_ref],
            Answer::ok(REMOTE_OID),
        )
        .with_git(
            &diff_args(REMOTE_OID),
            Answer::ok(&listing(&[(MODIFIED_STATUS, CHANGED_PATH)])),
        )
        .with_git(
            &[CAT_FILE, SIZE_FLAG, &changed],
            Answer::ok(&CHANGED_CONTENT.len().to_string()),
        )
        .with_git(&[CAT_FILE, BLOB, &changed], Answer::ok(CHANGED_CONTENT));

    let error = publish_with(&tools, Some(BRANCH)).unwrap_err();

    assert!(is_conflict(&error), "{error}");
    assert!(error.to_string().contains(REMOTE_MOVED));
    assert_eq!(tools.bodies().len(), 1);
}

/// A file over the per-file ceiling stops the publish before its content is read.
#[test]
fn an_oversized_file_is_refused_before_it_is_read() {
    let changed = object(CHANGED_PATH);
    let tools = on_branch(BRANCH)
        .with_git(
            &[MERGE_BASE, HEAD, &remote_ref(DEFAULT_BRANCH)],
            Answer::ok(BASE_OID),
        )
        .with_git(
            &diff_args(BASE_OID),
            Answer::ok(&listing(&[(MODIFIED_STATUS, CHANGED_PATH)])),
        )
        .with_git(&[CAT_FILE, SIZE_FLAG, &changed], Answer::ok(OVERSIZED));

    let error = publish_with(&tools, Some(BRANCH)).unwrap_err();

    assert!(is_conflict(&error), "{error}");
    assert!(error.to_string().contains(FILE_TOO_LARGE));
    assert!(!tools.called(&[CAT_FILE, BLOB, &changed]));
}

/// A GraphQL error reply is an upstream failure, so the flow may retry the publish.
#[test]
fn a_graphql_error_is_an_upstream_failure() {
    let tools = first_publish();
    {
        let mut replies = tools.graphql.lock().unwrap();
        replies.pop_back();
        replies.push_back(Answer::ok(ERRORS_REPLY));
    }

    let error = publish_with(&tools, Some(BRANCH)).unwrap_err();

    assert!(
        matches!(
            error.downcast_ref::<IntegrationError>(),
            Some(IntegrationError::Upstream(_))
        ),
        "{error}"
    );
    assert!(!tools.called(&[RESET, SOFT_FLAG, &remote_ref(BRANCH)]));
}
