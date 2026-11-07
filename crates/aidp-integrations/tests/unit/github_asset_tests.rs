use super::{allowed_asset_url, checked_repo, is_attachment, needs_token, signed_attachment_url};
use crate::squad::test_support::FakeRunner;

const RENDERED: &str = include_str!("../fixtures/github_asset/rendered_attachment.html");
const ATTACHMENT: &str =
    "https://github.com/user-attachments/assets/4d65bbc2-5e46-42b0-8d0e-30b7049a1e8c";
const REPO_ASSET: &str = "https://github.com/acme/app/assets/123/abc.png";
const RAW_IMAGE: &str = "https://raw.githubusercontent.com/acme/app/main/docs/flow.png";
const SIGNED_IMAGE: &str = "https://private-user-images.githubusercontent.com/1/2.png?jwt=x";
const SIGNED_FROM_FIXTURE: &str = "https://private-user-images.githubusercontent.com/165798309/667747798-4d65bbc2.png?jwt=header.payload.signature&v=1";
const AVATAR: &str = "https://avatars.githubusercontent.com/u/1?v=4";
const GITHUB_PAGE: &str = "https://github.com/acme/app/pull/12";
const OTHER_HOST: &str = "https://example.com/image.png";
const PLAIN_HTTP: &str = "http://github.com/user-attachments/assets/1";
const OTHER_PORT: &str = "https://github.com:8443/user-attachments/assets/1";
const LOOKALIKE_HOST: &str = "https://github.com.evil.example/user-attachments/assets/1";
const LOCAL_HOST: &str = "https://127.0.0.1/user-attachments/assets/1";
const NOT_A_URL: &str = "user-attachments/assets/1";
const REPO: &str = "acme/app";
const CONTEXT_FIELD: &str = "context=acme/app";
const BAD_REPOS: [&str; 4] = ["acme", "acme/app/extra", "acme/a b", "/app"];
const RENDERED_WITHOUT_IMAGE: &str = "<p>nothing</p>";

#[test]
fn github_hosted_files_are_allowed() {
    for url in [ATTACHMENT, REPO_ASSET, RAW_IMAGE, SIGNED_IMAGE, AVATAR] {
        assert!(allowed_asset_url(url).is_ok(), "{url}");
    }
}

#[test]
fn anything_else_is_refused() {
    for url in [
        GITHUB_PAGE,
        OTHER_HOST,
        PLAIN_HTTP,
        OTHER_PORT,
        LOOKALIKE_HOST,
        LOCAL_HOST,
        NOT_A_URL,
    ] {
        assert!(allowed_asset_url(url).is_err(), "{url}");
    }
}

#[test]
fn github_files_are_signed_and_only_raw_files_get_the_token() {
    for (url, attachment, token) in [
        (ATTACHMENT, true, false),
        (REPO_ASSET, true, false),
        (RAW_IMAGE, false, true),
        (SIGNED_IMAGE, false, false),
        (AVATAR, false, false),
    ] {
        let parsed = allowed_asset_url(url).unwrap();
        assert_eq!(is_attachment(&parsed), attachment, "{url}");
        assert_eq!(needs_token(&parsed), token, "{url}");
    }
}

#[test]
fn the_signed_address_is_read_from_the_rendering_in_the_repository_context() {
    let runner = FakeRunner::new(Box::new(|_| Ok(RENDERED.to_string())));
    let attachment = allowed_asset_url(ATTACHMENT).unwrap();
    let signed = signed_attachment_url(&attachment, REPO, &runner).unwrap();
    assert_eq!(signed.as_str(), SIGNED_FROM_FIXTURE);
    assert!(runner.calls()[0].iter().any(|arg| arg == CONTEXT_FIELD));
}

#[test]
fn a_rendering_without_a_signed_address_is_an_error() {
    let attachment = allowed_asset_url(ATTACHMENT).unwrap();
    let bare = FakeRunner::new(Box::new(|_| Ok(RENDERED_WITHOUT_IMAGE.to_string())));
    assert!(signed_attachment_url(&attachment, REPO, &bare).is_err());
    let unsigned = FakeRunner::new(Box::new(|_| Ok(format!("<img src=\"{ATTACHMENT}\">"))));
    assert!(signed_attachment_url(&attachment, REPO, &unsigned).is_err());
}

#[test]
fn the_repository_must_be_an_owner_name_slug() {
    assert_eq!(checked_repo(Some(REPO)).unwrap(), REPO);
    assert!(checked_repo(None).is_err());
    for repo in BAD_REPOS {
        assert!(checked_repo(Some(repo)).is_err(), "{repo}");
    }
}
