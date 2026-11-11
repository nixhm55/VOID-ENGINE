use std::{env, fs, path::PathBuf, process::Command};

fn resource_compiler() -> PathBuf {
    if let (Some(sdk_dir), Ok(sdk_version)) =
        (env::var_os("WindowsSdkDir"), env::var("WindowsSDKVersion"))
    {
        let sdk_arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("x86") => "x86",
            Ok("aarch64") => "arm64",
            _ => "x64",
        };
        let candidate = PathBuf::from(sdk_dir)
            .join("bin")
            .join(sdk_version.trim_matches(['\\', '/']))
            .join(sdk_arch)
            .join("rc.exe");
        if candidate.is_file() {
            return candidate;
        }
    }

    PathBuf::from("rc.exe")
}

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

    let rc = resource_compiler();
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource_file)
        .arg(&resource_script)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "Windows Resource Compiler (rc.exe) is required to embed the app icon; \
                 ensure the Windows SDK is installed and the MSVC developer environment is active ({rc:?}): {error}"
            )
        });
    assert!(status.success(), "rc.exe failed to compile the app icon");

    println!("cargo:rustc-link-arg-bin=void={}", resource_file.display());
}
