// Give the Windows front-end the same icon as the host executable. Without an
// embedded icon resource Windows falls back to a generic document glyph in the
// taskbar, window switcher and file properties.
use std::path::PathBuf;

/// `cargo build` on Windows does not always run inside a Visual Studio
/// developer prompt, so `rc.exe` may not be on PATH. Look where the SDK
/// installers put it.
fn windows_sdk_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RC_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let roots = [
        std::env::var_os("WindowsSdkDir").map(PathBuf::from),
        std::env::var_os("ProgramFiles(x86)")
            .map(|root| PathBuf::from(root).join("Windows Kits/10")),
    ];
    for root in roots.into_iter().flatten() {
        let bin = root.join("bin");
        let Ok(versions) = std::fs::read_dir(&bin) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = versions.flatten().map(|entry| entry.path()).collect();
        // Newest SDK first.
        versions.sort();
        for version in versions.into_iter().rev() {
            for host in ["x64", "x86", "arm64"] {
                let candidate = version.join(host).join("rc.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=../src/bitmaps/windows/icon.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../src/bitmaps/windows/icon.ico");
    resource.set("FileDescription", "Neo Aegisub subtitle editor");
    resource.set("ProductName", "Neo Aegisub");
    resource.set("OriginalFilename", "aegisub-gpui-prototype.exe");
    if let Some(rc) = windows_sdk_resource_compiler() {
        if let Some(directory) = rc.parent().and_then(|path| path.to_str()) {
            resource.set_toolkit_path(directory);
        }
    }
    // rustc already embeds its own manifest; adding another one makes CVTRES
    // fail with "duplicate resource. type:MANIFEST".
    if let Err(error) = resource.compile() {
        panic!(
            "failed to embed the Windows icon: {error}. \
             Set RC_PATH to the Windows SDK rc.exe if it is not installed in the default location."
        );
    }
}
