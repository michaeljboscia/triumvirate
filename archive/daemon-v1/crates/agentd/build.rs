use std::path::Path;
use std::process::Command;

fn main() {
    let frontend_dir = Path::new("../../frontend");
    let dist_dir = frontend_dir.join("dist");

    println!("cargo:rerun-if-changed=../../frontend/src");
    println!("cargo:rerun-if-changed=../../frontend/package.json");
    println!("cargo:rerun-if-changed=../../frontend/package-lock.json");
    println!("cargo:rerun-if-changed=../../frontend/vite.config.ts");

    if std::env::var("TRIUMVIRATE_SKIP_FRONTEND_BUILD").ok().as_deref() == Some("1") {
        return;
    }

    let status = Command::new("npm")
        .arg("run")
        .arg("build")
        .current_dir(frontend_dir)
        .status()
        .expect("failed to execute npm; install Node.js and npm");

    if !status.success() {
        panic!("frontend build failed; run `cd daemon/frontend && npm run build`");
    }

    if !dist_dir.exists() {
        panic!("frontend dist directory missing after build");
    }
}
