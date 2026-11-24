//! Started by a browser to carry the Afterimage extension's messages. See `bridge.rs`.
fn main() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let socket = std::path::Path::new(&home).join("Library/Application Support/Afterimage/bridge.sock");
    // The browser is whoever started us.
    let _ = afterimage_core::bridge::forward(&mut std::io::stdin().lock(), &socket, std::os::unix::process::parent_id());
}
