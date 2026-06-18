/// Integration tests for the `omnicompress` CLI binary.
///
/// Verifies that `omnicompress compress <file>` prints valid JSON
/// with tokens_before, tokens_after, and tokens_saved.
#[test]
fn cli_compress_file_reports_savings() {
    use std::process::Command;

    // Build a JSON array large enough to trigger compression
    // (needs to exceed min_chars_to_compress = 600)
    let big = "[".to_string()
        + &(0..60)
            .map(|i| format!(r#"{{"id":{i},"v":"{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
        + "]";

    let f = std::env::temp_dir().join("omnicompress_cli_test.json");
    std::fs::write(&f, &big).expect("write test file");

    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress"))
        .arg("compress")
        .arg(&f)
        .output()
        .expect("run omnicompress binary");

    assert!(
        out.status.success(),
        "binary exited with non-zero status: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tokens_before"),
        "stdout missing tokens_before: {stdout}"
    );
    assert!(
        stdout.contains("tokens_after"),
        "stdout missing tokens_after: {stdout}"
    );
    assert!(
        stdout.contains("tokens_saved"),
        "stdout missing tokens_saved: {stdout}"
    );

    // Clean up
    let _ = std::fs::remove_file(&f);
}
