use std::path::Path;
use std::process::Command;

fn main() {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web");
    let dist_file = web_dir.join("dist/decrypt.js");

    // Only rebuild if the output doesn't exist or source files changed
    println!("cargo::rerun-if-changed=../web/src/");
    println!("cargo::rerun-if-changed=../web/build.mjs");
    println!("cargo::rerun-if-changed=../web/package.json");

    if dist_file.exists() {
        return;
    }

    eprintln!("web/dist/decrypt.js not found, building web assets...");

    // Install npm dependencies if needed
    let node_modules = web_dir.join("node_modules");
    if !node_modules.exists() {
        let status = Command::new("npm")
            .arg("install")
            .current_dir(&web_dir)
            .status()
            .expect("failed to run 'npm install' — is npm installed?");
        assert!(status.success(), "npm install failed");
    }

    // Build web assets
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .status()
        .expect("failed to run 'npm run build' — is npm installed?");
    assert!(status.success(), "web build failed");
}
