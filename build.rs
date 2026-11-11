use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=branding/app.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("branding/app.ico");
    if !icon.is_file() {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let resource_script = out_dir.join("void_icon.rc");
    let resource_file = out_dir.join("void_icon.res");
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    fs::write(
        &resource_script,
        format!("IDI_ICON1 ICON \"{icon_path}\"\n"),
    )
    .expect("failed to write the Windows icon resource script");

    let status = Command::new("rc.exe")
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource_file)
        .arg(&resource_script)
        .status()
        .expect("Windows Resource Compiler (rc.exe) is required to embed the app icon");
    assert!(status.success(), "rc.exe failed to compile the app icon");

    println!("cargo:rustc-link-arg-bin=void={}", resource_file.display());
}
