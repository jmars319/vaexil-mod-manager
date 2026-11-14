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
; Files in use are set aside by [Code] below instead of closing what holds
; them: an AI app's session runs vmm.exe as its MCP server for as long as it
; is open, and the window may be mid-deploy.
CloseApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\vaexil-mod-manager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\vmm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; An explicit IconFilename, so Windows refreshes the icon when the exe changes.
Name: "{autoprograms}\Vaexil Mod Manager"; Filename: "{app}\vaexil-mod-manager.exe"; IconFilename: "{app}\vaexil-mod-manager.exe"
Name: "{autodesktop}\Vaexil Mod Manager"; Filename: "{app}\vaexil-mod-manager.exe"; IconFilename: "{app}\vaexil-mod-manager.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\vaexil-mod-manager.exe"; Description: "Open Vaexil Mod Manager"; Flags: nowait postinstall skipifsilent; Check: not WindowWasOpen
; The app's own update runs this installer silently with /relaunch=1 after
; closing itself, and expects to be reopened.
Filename: "{app}\vaexil-mod-manager.exe"; Flags: nowait; Check: Relaunch

[UninstallDelete]
; Copies set aside while they were running (see [Code]). Otherwise this
; version writes nothing outside its install folder, and never writes inside
; HITMAN's folder or Simple Mod Framework's, so uninstall has nothing else to
; clean up.
Type: files; Name: "{app}\*.old-*"

[Code]
var
  WindowOpen: Boolean;

function WindowWasOpen: Boolean;
begin
  Result := WindowOpen;
end;

function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;

{ Deletes copies set aside by an earlier install, where nothing runs them now. }
procedure DeleteOldCopies;
var
  Found: TFindRec;
begin
  if FindFirst(ExpandConstant('{app}\*.old-*'), Found) then
  try
    repeat
      DeleteFile(ExpandConstant('{app}\') + Found.Name);
    until not FindNext(Found);
  finally
    FindClose(Found);
  end;
end;

{ Windows won't replace or delete a running exe, but it will rename one.
  So a file in use is renamed aside, keeps running from its new name, and
  the new file goes in under the real name. Returns whether it was in use. }
function SetAsideIfInUse(Name: String): Boolean;
var
  Path: String;
begin
  Result := False;
  Path := ExpandConstant('{app}\') + Name;
  if FileExists(Path) and not DeleteFile(Path) then
  begin
    Result := True;
    if not RenameFile(Path, Path + '.old-' + GetDateTimeString('yyyymmddhhnnss', #0, #0)) then
      Log('Could not set aside ' + Path + '; Setup will report it if the copy fails.');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    DeleteOldCopies;
    { An AI app's open session keeps its vmm.exe; new sessions get the new one. }
    SetAsideIfInUse('vmm.exe');
    WindowOpen := SetAsideIfInUse('vaexil-mod-manager.exe');
  end;
  if (CurStep = ssDone) and WindowOpen and not WizardSilent then
    MsgBox('Vaexil Mod Manager is open. It keeps running the version it started with; close and reopen it to use the new one.', mbInformation, MB_OK);
end;
