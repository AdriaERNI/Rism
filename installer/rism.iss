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
; default state is checked (there is no "checked" flag in Inno — only "unchecked")
Name: "modifypath"; Description: "Add {#MyAppName} to PATH"; GroupDescription: "Integration:"

[Files]
Source: "{#AppSource}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName} documentation"; Filename: "{#MyAppURL}"

[Code]
{ PATH via [Code], not [Registry]: Inno cannot safely restore a *Path* value }
{ on uninstall: olddata round-trips broke the CI absence check. Pattern       }
{ ported from Prism prism.iss EnvAddPath/EnvRemovePath (Store-tested).         }
{ NOTE: inside [Code], a leading ';' is NOT a comment — use brace blocks.    }
procedure EnvAddPath(Path: string);
var
  Root: Integer;
  Key: string;
  Paths: string;
begin
  if IsAdminInstallMode then
  begin Root := HKLM; Key := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'; end
  else
  begin Root := HKCU; Key := 'Environment'; end;
  if not RegQueryStringValue(Root, Key, 'Path', Paths) then
    Paths := '';
  if Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';') > 0 then
    exit;
  if Paths <> '' then
    Paths := Paths + ';';
  Paths := Paths + Path;
  RegWriteStringValue(Root, Key, 'Path', Paths);
end;

procedure RemovePathFrom(Root: Integer; Key: string; Path: string);
var
  Paths: string;
  P: Integer;
begin
  if not RegQueryStringValue(Root, Key, 'Path', Paths) then
    exit;
  P := Pos(';' + Uppercase(Path), ';' + Uppercase(Paths));
  if P > 0 then
    Delete(Paths, P - 1, Length(Path) + 1)
  else if Uppercase(Copy(Paths, 1, Length(Path))) = Uppercase(Path) then
  begin
    { installed as the very first entry }
    Delete(Paths, 1, Length(Path) + 1);
  end
  else
    exit;
  RegWriteStringValue(Root, Key, 'Path', Paths);
end;

procedure EnvRemovePath(Path: string);
begin
  { scrub both hives: an admin install wrote HKLM, per-user wrote HKCU }
  RemovePathFrom(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', Path);
  RemovePathFrom(HKCU, 'Environment', Path);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('modifypath') then
    EnvAddPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    EnvRemovePath(ExpandConstant('{app}'));
end;

[Run]
Filename: "{app}\{#MyAppExeName}"; Parameters: "--version"; \
  Description: "Verify installation (runs rism --version)"; \
  Flags: postinstall skipifsilent nowait runascurrentuser
