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

/// Integration test for `omnicompress eval <dir>`.
///
/// Writes a temp dir with one JSON file (a `Vec<Block>` with a big tool block)
/// and asserts the output contains a ratio and `roundtrip_ok`.
#[test]
fn cli_eval_dir_reports_ratio_and_roundtrip() {
    use std::process::Command;

    // Build a JSON array large enough to trigger compression (>600 chars, >20 items).
    let big_json_content = "[".to_string()
        + &(0..60)
            .map(|i| format!(r#"{{"id":{i},"name":"item-{i}","score":{i}.0,"desc":"padding-{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
        + "]";

    // Construct a Vec<Block> as JSON: one big tool block + 6 recent prose blocks.
    // Block serialisation: {"role":"User","content":"...","tool_name":"Bash"}
    let mut blocks = vec![
        serde_block("User", &big_json_content, Some("Bash")),
    ];
    for i in 0..6 {
        blocks.push(serde_block("Assistant", &format!("ok {i}"), None));
    }
    let sample_json = format!("[{}]", blocks.join(","));

    // Write a temp dir with one sample file.
    let tmp_dir = std::env::temp_dir().join("omnicompress_eval_test_dir");
    std::fs::create_dir_all(&tmp_dir).expect("create temp dir");
    let sample_path = tmp_dir.join("sample.json");
    std::fs::write(&sample_path, &sample_json).expect("write sample file");

    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress"))
        .arg("eval")
        .arg(&tmp_dir)
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
        stdout.contains("ratio"),
        "stdout missing ratio: {stdout}"
    );
    assert!(
        stdout.contains("roundtrip_ok"),
        "stdout missing roundtrip_ok: {stdout}"
    );

    // Clean up
    let _ = std::fs::remove_file(&sample_path);
    let _ = std::fs::remove_dir(&tmp_dir);
}

/// Helper: serialise a Block as a JSON object string.
fn serde_block(role: &str, content: &str, tool_name: Option<&str>) -> String {
    let escaped_content = content.replace('\\', "\\\\").replace('"', "\\\"");
    match tool_name {
        Some(t) => format!(
            r#"{{"role":"{role}","content":"{escaped_content}","tool_name":"{t}"}}"#
        ),
        None => format!(
            r#"{{"role":"{role}","content":"{escaped_content}","tool_name":null}}"#
        ),
    }
}
