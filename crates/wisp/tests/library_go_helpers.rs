use std::{fs, process::Command};

#[test]
fn private_go_transport_helpers_preserve_ownership_and_text() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("go.mod"),
        "module helpers.local/check\n\ngo 1.27.0\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("values.go"),
        format!(
            "{}\n{}\n{}",
            include_str!("../assets/templates/library-go-values.go"),
            include_str!("../assets/templates/library-go-resources.go"),
            include_str!("../assets/templates/library-go-error-context.go")
        ),
    )
    .unwrap();
    fs::write(
        temp.path().join("values_test.go"),
        include_str!("../../../tests/library-chain/general/go_helpers_test.go"),
    )
    .unwrap();
    let output = Command::new("go")
        .current_dir(temp.path())
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GOTOOLCHAIN", "local")
        .args(["test", "-race", "-count=1", "-v", "./..."])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
