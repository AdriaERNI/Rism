; Rism — Inno Setup 6 script
; Built by ci.yml (tag) and .github/workflows/windows-installer.yml (gate).
; Flags: /DAppVersion=X.Y.Z /DAppSource=<dir containing rism.exe>
; Machine-wide install into {autopf}\Rism with PATH registration — the layout
; the CI silent install/upgrade/uninstall contract verifies, and what the
; Microsoft Store Win32 submission ships.

#define MyAppName "Rism"
#define MyAppPublisher "Adria Sanchez"
#define MyAppURL "https://adriaerni.github.io/Rism/"
#define MyAppExeName "rism.exe"

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef AppSource
  #define AppSource "..\target\release"
#endif

[Setup]
; Stable AppId: never change once shipped (upgrade detection keys on it).
AppId={{D1C0A9E4-2B6F-4A3D-8E57-0C6B9F1A4E3D}
AppName={#MyAppName}
AppVersion={#AppVersion}
AppVerName={#MyAppName} {#AppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
LicenseFile=..\LICENSE
OutputDir=Output
OutputBaseFilename=rism-{#AppVersion}-setup
SetupIconFile=assets\logo.ico
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
WizardImageFile=assets\wizard-image.bmp
WizardSmallImageFile=assets\wizard-small.bmp
ChangesEnvironment=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "modifypath"; Description: "Add {#MyAppName} to PATH"; GroupDescription: "Integration:"; Flags: checked

[Files]
Source: "{#AppSource}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName} documentation"; Filename: "{#MyAppURL}"

[Registry]
; PATH registration behind the modifypath task; {autopf} resolves per-user or
; per-machine to match PrivilegesRequired at install time.
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
  ValueData: "{app};{olddata}"; Tasks: modifypath; Check: NotOnPathAlready
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
  ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
  Tasks: modifypath; Check: NotOnPathAlreadyAndNeedsHKLM

[Code]
function NotOnPathAlready: Boolean;
var
  Paths: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Paths) then
    Paths := '';
  Result := Pos(Lowercase(ExpandConstant('{app}')), Lowercase(Paths)) = 0;
end;

function NotOnPathAlreadyAndNeedsHKLM: Boolean;
var
  Paths: string;
begin
  if IsAdminInstallMode then
  begin
    if not RegQueryStringValue(HKLM,
        'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
        'Path', Paths) then
      Paths := '';
    Result := Pos(Lowercase(ExpandConstant('{app}')), Lowercase(Paths)) = 0;
  end
  else
    Result := False;  // per-user mode: HKCU entry above is the one
end;

[Run]
Filename: "{app}\{#MyAppExeName}"; Parameters: "--version"; \
  Description: "Verify installation (runs rism --version)"; \
  Flags: postinstall skipifsilent nowait runascurrentuser
