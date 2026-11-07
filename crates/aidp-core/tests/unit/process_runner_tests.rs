use super::*;

#[test]
fn scripted_runner_output_returns_canned_bytes() {
    let runner = ScriptedProcessRunner::new().with_output(ScriptedOutput {
        status_code: 0,
        stdout: b"hello".to_vec(),
        stderr: Vec::new(),
    });
    let out = runner.output(&ProcessSpec::new("anything")).unwrap();
    assert_eq!(out.stdout, b"hello");
    assert!(out.status.success());
}

#[test]
fn scripted_runner_spawn_replays_stdout_on_the_fake_child() {
    let runner = ScriptedProcessRunner::new().with_spawn_stdout(b"line1\nline2\n".to_vec());
    let mut child = runner.spawn(&ProcessSpec::new("claude")).unwrap();
    let mut reader = child.take_stdout().expect("stdout must be present");
    let mut buf = String::new();
    reader.read_to_string(&mut buf).unwrap();
    assert_eq!(buf, "line1\nline2\n");
    assert_eq!(child.wait().unwrap(), 0);
}

#[test]
fn process_spec_builder_accumulates_args() {
    let spec = ProcessSpec::new("git").arg("status").arg("--porcelain");
    assert_eq!(spec.args, vec!["status", "--porcelain"]);
}
