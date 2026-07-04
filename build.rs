//! Build the embedded web portal before compiling the crate.
//!
//! The Rust code `include_str!`s `web/dist/index.html`, which is produced by the
//! Vite build in `web/`. This runs that build so the SPA is always current.
//!
//! Note: pnpm 11 blocks a plain `pnpm build` (its pre-run gate fails on esbuild's
//! "ignored build script"), so we invoke Vite directly via Node once deps are
//! installed — that's the reliable non-interactive path.

use std::path::Path;
use std::process::Command;

fn main() {
    let web = Path::new("web");
    if !web.join("package.json").exists() {
        // No frontend project (e.g. a source tarball without web/) — rely on a
        // committed dist/index.html if present, else fail the include_str! later
        // with a clear compile error.
        return;
    }

    // Rebuild the SPA when any of its inputs change.
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/vite.config.js");

    // Skip the frontend build if explicitly requested (e.g. CI already built it,
    // or to build the Rust crate offline against an existing dist/).
    if std::env::var("TIDAL_SUBSONIC_SKIP_WEB_BUILD").is_ok() {
        println!("cargo:warning=Skipping web build (TIDAL_SUBSONIC_SKIP_WEB_BUILD set)");
        return;
    }

    // Install dependencies if node_modules is missing.
    if !web.join("node_modules").join("vite").exists() {
        run(
            web,
            pnpm(),
            &["install", "--config.confirmModulesPurge=false"],
            "pnpm install",
        );
    }

    // Build via Vite directly (bypasses pnpm 11's run wrapper / esbuild gate).
    run(
        web,
        "node",
        &["node_modules/vite/bin/vite.js", "build"],
        "vite build",
    );

    if !web.join("dist").join("index.html").exists() {
        panic!("web build did not produce web/dist/index.html");
    }
}

/// pnpm binary name (Windows uses pnpm.cmd).
fn pnpm() -> &'static str {
    if cfg!(windows) {
        "pnpm.cmd"
    } else {
        "pnpm"
    }
}

fn run(dir: &Path, program: &str, args: &[&str], label: &str) {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("{label} failed with status {s}"),
        Err(e) => panic!(
            "{label} could not run ({e}). The web portal build needs Node + pnpm \
             installed. Install them, or set TIDAL_SUBSONIC_SKIP_WEB_BUILD=1 and \
             provide a prebuilt web/dist/index.html."
        ),
    }
}
