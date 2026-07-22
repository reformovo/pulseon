#[cfg(target_os = "macos")]
fn main() {
    eprintln!("pulseon-viewer application shell is not available until Phase 3C");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("pulseon-viewer is unsupported on this platform");
}
