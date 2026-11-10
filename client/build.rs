//! Embeds the Windows version info and application manifest into the exe.
//!
//! An exe with no publisher, product name or manifest looks anonymous to
//! Windows (Properties → Details is empty) and to antivirus heuristics. The
//! manifest also states that the program never asks for elevation.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = env!("CARGO_PKG_VERSION");
        let manifest = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Lawful.LawfulOverlay" version="{version}.0" processorArchitecture="*"/>
  <description>LawfulOverlay</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <!-- Windows 10 and 11 -->
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>
"#
        );

        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "LawfulOverlay")
            .set("FileDescription", "LawfulOverlay")
            .set("CompanyName", "LawfulOverlay")
            .set("LegalCopyright", "Copyright (c) 2025 LawfulOverlay. MIT License.")
            .set("OriginalFilename", "LawfulOverlay.exe")
            .set("InternalName", "LawfulOverlay")
            .set("Comments", "https://github.com/Lawful831/lawfuloverlay")
            .set_manifest(&manifest);
        res.compile().expect("embed Windows resources");
    }
}
