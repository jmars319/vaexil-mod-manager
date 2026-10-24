; Per-user installer for Vaexil Mod Manager. Needs no administrator rights.
; Built by scripts/package.ps1, which passes AppVersion and SourceDir.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\staging"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{B7D9C2A4-5E3F-4C61-8F2A-9D4E7A1C3B58}
AppName=Vaexil Mod Manager
AppVersion={#AppVersion}
AppPublisher=Vaexil Mod Manager contributors
AppPublisherURL=https://github.com/jmars319/vaexil-mod-manager
AppSupportURL=https://github.com/jmars319/vaexil-mod-manager/issues
DefaultDirName={autopf}\Vaexil Mod Manager
DefaultGroupName=Vaexil Mod Manager
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile={#SourceDir}\LICENSE.txt
OutputDir={#OutputDir}
OutputBaseFilename=VaexilModManager-{#AppVersion}-setup
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\vaexil-mod-manager.exe
UninstallDisplayName=Vaexil Mod Manager
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\vaexil-mod-manager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; An explicit IconFilename, so Windows refreshes the icon when the exe changes.
Name: "{autoprograms}\Vaexil Mod Manager"; Filename: "{app}\vaexil-mod-manager.exe"; IconFilename: "{app}\vaexil-mod-manager.exe"
Name: "{autodesktop}\Vaexil Mod Manager"; Filename: "{app}\vaexil-mod-manager.exe"; IconFilename: "{app}\vaexil-mod-manager.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\vaexil-mod-manager.exe"; Description: "Open Vaexil Mod Manager"; Flags: nowait postinstall skipifsilent

; This version writes nothing outside its install folder, and never writes
; inside HITMAN's folder or Simple Mod Framework's, so uninstall has nothing
; else to clean up.
