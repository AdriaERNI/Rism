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
    // FileVersion/ProductVersion STRINGS (what VersionInfo.FileVersion reads
    // back in PowerShell) are NOT derived from FixedFileInfo — set explicitly.
    res.set("FileVersion", env!("CARGO_PKG_VERSION"));
    res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
    if let Err(e) = res.compile() {
        println!("cargo:warning=winresource: {e}");
    }
}

#[cfg(not(windows))]
fn main() {}
