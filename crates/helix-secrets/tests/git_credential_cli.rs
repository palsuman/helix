use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn helper_reads_the_git_action_from_argv() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_helix-git-credential"))
        .arg("unsupported-action")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"protocol=https\nhost=example.com\n\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported-action"), "{stderr}");
    assert!(!stderr.contains("protocol=https"), "{stderr}");
}
