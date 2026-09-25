// Windows builds embed a VERSIONINFO resource so Explorer, the installer
// verification (windows-installer.yml) and Store certification can read
// FileVersion/ProductVersion without running the exe. No-op on other hosts.
#[cfg(windows)]
fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "Rism");
    res.set("FileDescription", "Prism with Rust — IRIS CLI + MCP server");
    res.set("CompanyName", "Adria Sanchez");
    res.set("LegalCopyright", "AGPL-3.0-or-later");
    // winresource picks up CARGO_PKG_VERSION for the fixedFileInfo.
    if let Err(e) = res.compile() {
        println!("cargo:warning=winresource: {e}");
    }
}

#[cfg(not(windows))]
fn main() {}
