use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=frontend/index.html");
    println!("cargo:rerun-if-changed=frontend/vite.config.ts");
    println!("cargo:rerun-if-changed=frontend/package.json");

    let frontend_dir = std::path::Path::new("frontend");

    // Skip frontend build if npm is not available (e.g. CI without node)
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    let status = Command::new(npm)
        .arg("run")
        .arg("build")
        .current_dir(frontend_dir)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("frontend build failed with {s}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("cargo:warning=npm not found, skipping frontend build");
        }
        Err(e) => panic!("failed to run npm: {e}"),
    }
}
