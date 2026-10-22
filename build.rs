// Windows: embed product name/version (required for signing) and a manifest that never asks for admin rights.
fn main() {
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set("ProductName", "Nexus-Mod-Duplication-Cleanup")
        .set("FileDescription", "Nexus Mod Duplication Cleanup")
        .set("OriginalFilename", "NexusModCleanup.exe")
        .set_manifest(r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges>
    <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
  </requestedPrivileges></security></trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings>
    <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
  </windowsSettings></application>
</assembly>"#)
        .compile()
        .expect("embed Windows resources");
}
