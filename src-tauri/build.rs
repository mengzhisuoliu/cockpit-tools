#[cfg(target_os = "macos")]
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(target_os = "macos")]
use swift_rs::SwiftLinker;

#[cfg(target_os = "macos")]
fn link_macos_swift_runtime_rpaths() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

#[cfg(target_os = "macos")]
fn copy_if_changed(source: &Path, destination: &Path) {
    let next = fs::read(source)
        .unwrap_or_else(|error| panic!("failed to read icon source {}: {error}", source.display()));

    if let Ok(current) = fs::read(destination) {
        if current == next {
            return;
        }
    }

    fs::write(destination, next).unwrap_or_else(|error| {
        panic!(
            "failed to write native menu icon {}: {error}",
            destination.display()
        )
    });
}

#[cfg(target_os = "macos")]
fn sync_macos_native_menu_icons() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let source_dir = manifest_dir.join("../src/assets/icons");
    let destination_dir =
        manifest_dir.join("native/macos-native-menu/Sources/MacosNativeMenuSwift/Resources");

    fs::create_dir_all(&destination_dir).unwrap_or_else(|error| {
        panic!(
            "failed to create native menu resource dir {}: {error}",
            destination_dir.display()
        )
    });

    let mappings = [
        ("antigravity-menu.png", "antigravity-menu.png"),
        ("codex.svg", "codex.svg"),
        ("cursor-menu.png", "cursor-menu.png"),
        ("gemini-menu.png", "gemini-menu.png"),
        ("github-copilot.svg", "github-copilot.svg"),
        ("kiro-menu.png", "kiro-menu.png"),
        ("windsurf.svg", "windsurf.svg"),
        ("zed.png", "zed.png"),
        ("codebuddy.png", "codebuddy.png"),
        ("qoder.png", "qoder.png"),
        ("trae.png", "trae.png"),
        ("workbuddy.png", "workbuddy.png"),
    ];

    for (source_name, destination_name) in mappings {
        let source = source_dir.join(source_name);
        println!("cargo:rerun-if-changed={}", source.display());
        copy_if_changed(&source, &destination_dir.join(destination_name));
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "macos")]
    {
        sync_macos_native_menu_icons();
        SwiftLinker::new("12.0")
            .with_package("MacosNativeMenuSwift", "native/macos-native-menu")
            .link();
        link_macos_swift_runtime_rpaths();
    }

    tauri_build::build()
}
