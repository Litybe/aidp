use super::*;
use aidp_testkit::{EnvGuard, TempWorkspace};

/// A fake `gh` (a `.bat`/shell script) placed at the front of `PATH` that
/// dumps its argv to the file named by `AIDP_TEST_ARGV_OUT`, byte-compares
/// the file passed after `--body-file` against `AIDP_TEST_EXPECTED`
/// (recording the verdict in `AIDP_TEST_CMP_OUT`) while that temp file
/// still exists, then prints a fake PR URL. No network access. Asserts
/// `--body-file` is passed, `--body` is not, and the temp file's content
/// was byte-for-byte identical to the supplied body.
#[test]
fn pr_create_passes_body_file_not_body_arg() {
    let workspace = TempWorkspace::new("ghpath");
    let dir = workspace.path().to_path_buf();
    let argv_out = dir.join("argv.txt");
    let expected = dir.join("expected.txt");
    let cmp_out = dir.join("cmp.txt");

    let body = "line one\nligne deux — café\n";
    std::fs::write(&expected, body).unwrap();

    #[cfg(windows)]
    let fake = dir.join("gh.bat");
    #[cfg(windows)]
    std::fs::write(
        &fake,
        "@echo off\r\n\
         setlocal enabledelayedexpansion\r\n\
         > \"%AIDP_TEST_ARGV_OUT%\" (for %%a in (%*) do echo %%a)\r\n\
         set BODYFILE=\r\n\
         :loop\r\n\
         if \"%~1\"==\"\" goto done\r\n\
         if \"%~1\"==\"--body-file\" set BODYFILE=%~2\r\n\
         shift\r\n\
         goto loop\r\n\
         :done\r\n\
         if not \"%BODYFILE%\"==\"\" (\r\n\
           fc /A \"%AIDP_TEST_EXPECTED%\" \"%BODYFILE%\" >nul 2>&1\r\n\
           if errorlevel 1 (echo NOMATCH>\"%AIDP_TEST_CMP_OUT%\") else (echo MATCH>\"%AIDP_TEST_CMP_OUT%\")\r\n\
         )\r\n\
         echo https://example.invalid/pr/1\r\n",
    )
    .unwrap();

    #[cfg(not(windows))]
    let fake = dir.join("gh");
    #[cfg(not(windows))]
    {
        std::fs::write(
            &fake,
            "#!/bin/sh\n\
             for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$AIDP_TEST_ARGV_OUT\"\n\
             bodyfile=\"\"\n\
             while [ \"$#\" -gt 0 ]; do\n\
               if [ \"$1\" = \"--body-file\" ]; then bodyfile=\"$2\"; fi\n\
               shift\n\
             done\n\
             if [ -n \"$bodyfile\" ]; then\n\
               if cmp -s \"$AIDP_TEST_EXPECTED\" \"$bodyfile\"; then echo MATCH > \"$AIDP_TEST_CMP_OUT\"; else echo NOMATCH > \"$AIDP_TEST_CMP_OUT\"; fi\n\
             fi\n\
             echo https://example.invalid/pr/1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
    }

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let new_path = format!("{}{sep}{}", dir.display(), old_path.to_string_lossy());

    let _env_guard = EnvGuard::with(&[
        ("PATH", Some(&new_path)),
        ("AIDP_TEST_ARGV_OUT", argv_out.to_str()),
        ("AIDP_TEST_EXPECTED", expected.to_str()),
        ("AIDP_TEST_CMP_OUT", cmp_out.to_str()),
    ]);

    let result = pr_create("a title", body, None);

    let url = result.expect("pr_create should succeed against the fake gh");
    assert_eq!(url, "https://example.invalid/pr/1");

    let argv_text = std::fs::read_to_string(&argv_out).expect("fake gh should have captured argv");
    assert!(
        argv_text.contains("--body-file"),
        "argv missing --body-file:\n{argv_text}"
    );
    assert!(
        !argv_text.lines().any(|l| l == "--body"),
        "argv should not carry a bare --body flag:\n{argv_text}"
    );

    let cmp = std::fs::read_to_string(&cmp_out).unwrap_or_default();
    assert_eq!(
        cmp.trim(),
        "MATCH",
        "temp body file did not match the supplied body byte for byte"
    );
}
