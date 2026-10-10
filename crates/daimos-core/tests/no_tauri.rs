use std::process::Command;

/// `daimos-core` must stay headless: a `tauri` entry anywhere in its tree is a layering bug.
#[test]
fn core_does_not_depend_on_tauri() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args(["tree", "-p", "daimos-core", "--prefix", "none", "--edges", "normal,build"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let tree = String::from_utf8_lossy(&output.stdout);
    let offenders: Vec<&str> = tree.lines().filter(|l| l.starts_with("tauri")).collect();
    assert!(offenders.is_empty(), "daimos-core depends on tauri: {offenders:?}");
}
