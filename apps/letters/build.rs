//! Gives the Windows program what the system reads from its file: the icon, the version, the name Task
//! Manager shows, and the manifest (`windows/letters.manifest`). Other systems read none of it.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(windows)]
    resources();
}

/// Only where the program is built on Windows for Windows: the system's resource compiler does the work.
#[cfg(windows)]
fn resources() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    println!("cargo::rerun-if-changed=windows/letters.manifest");
    println!("cargo::rerun-if-changed=../../assets/icons/letters.ico");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../assets/icons/letters.ico")
        .set_manifest_file("windows/letters.manifest")
        .set("ProductName", "Letters")
        .set("FileDescription", "Letters")
        .set("CompanyName", "Sierra Softworks")
        .set("InternalName", "letters")
        .set("OriginalFilename", "letters.exe");
    resource.compile_for("letters").expect("the Windows resources compile");
}
