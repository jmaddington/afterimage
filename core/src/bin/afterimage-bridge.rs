//! Started by a browser to carry the Afterimage extension's messages. See `bridge.rs`.
fn main() {
    // AFTERIMAGE_HOME: the same override the app honours, for trying things against a throwaway library.
    let dir = match (std::env::var_os("AFTERIMAGE_HOME"), std::env::var_os("HOME")) {
        (Some(d), _) => std::path::PathBuf::from(d),
        (None, Some(h)) => std::path::Path::new(&h).join("Library/Application Support/Afterimage"),
        _ => return,
    };
    let socket = dir.join("bridge.sock");
    // The browser is whoever started us.
    let _ = afterimage_core::bridge::forward(&mut std::io::stdin().lock(), std::io::stdout(), &socket, std::os::unix::process::parent_id());
}
