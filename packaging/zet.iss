; zet — installer script for Inno Setup 6.3 or newer.
;
; Build it with the version the binary reports, or the two disagree:
;
;     ISCC.exe /DVersion=0.2.0 /O"..\dist" ..\packaging\zet.iss
;
; `packaging/README.md` explains every choice below. The short version: this installs
; per user, into a directory that needs no administrator, and never raises a UAC prompt.

#ifndef Version
  ; Only reachable when someone compiles this by hand without /DVersion. The release
  ; workflow always passes one, and a version of 0.0.0 in Apps & features is a visible
  ; enough sign that this branch was taken.
  #define Version "0.0.0"
#endif

#define AppName "zet"
#define AppUrl "https://github.com/asterxsk/zet"
#define ExeName "zet.exe"

[Setup]
; The AppId is what makes an upgrade an upgrade rather than a second installation: it
; names the uninstall registry key and matches the uninstall log. It must never change.
AppId={{6C3259ED-B491-4243-B3B6-553EE8E94B77}
AppName={#AppName}
AppVersion={#Version}
VersionInfoVersion={#Version}
AppPublisher=asterxsk
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases

; No UAC, ever. `lowest` means Setup declines elevation even when the account could
; have it. Leaving `PrivilegesRequiredOverridesAllowed` unset is the other half: it
; removes the "install for all users" option and the /ALLUSERS switch, so nothing can
; talk the installer into machine-wide mode later.
PrivilegesRequired=lowest

; The per-user location, and no page offering to change it — the answer was decided
; when the install mode was, and a directory page here would only be able to offer
; locations that need the elevation this installer refuses to ask for.
DefaultDirName={localappdata}\Programs\zet
DisableDirPage=yes
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
DefaultGroupName={#AppName}

; ConPTY, which zet's pty layer requires, is Windows 10 1809 and later.
MinVersion=10.0.17763
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; `ChangesEnvironment=yes` is what makes the installer broadcast WM_SETTINGCHANGE after
; writing PATH. Without it the new entry is invisible to every already-running program
; until the user signs out, which reads as "the installer did not work".
ChangesEnvironment=yes

; Not `zet-setup` alone: any file called `setup.exe` is shimmed by Windows application
; compatibility, which loads extra DLLs into it. The release workflow names the output
; `zet-<version>-setup.exe`, which is not that name.
OutputBaseFilename=zet-setup
SetupIconFile=zet.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern dynamic

UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#ExeName}
UninstallFilesDir={app}
CreateUninstallRegKey=yes
Uninstallable=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startmenu"; Description: "Create a &Start Menu shortcut"; GroupDescription: "Integration:"; Flags: checkedonce
Name: "addtopath"; Description: "Add zet to my &PATH, so `zet` works from any directory"; GroupDescription: "Integration:"; Flags: unchecked
Name: "ctxmenu"; Description: "Add ""Open zet here"" to the folder right-click menu"; GroupDescription: "Integration:"; Flags: unchecked

[Files]
Source: "..\target\release\{#ExeName}"; DestDir: "{app}"; Flags: ignoreversion
; The binary statically links its own fonts, so the notices that cover them have to
; travel with it rather than with a separate font file.
Source: "..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\{#ExeName}"; Tasks: startmenu

[Registry]
; "Open zet here" on the folder background and on a folder itself. Both are registered
; because they are different gestures and Windows keeps them in different keys.
;
; HKCU\Software\Classes rather than HKCR: HKCR writes land in HKLM and need the
; elevation this installer does not ask for.
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\zet"; ValueType: string; \
    ValueName: ""; ValueData: "Open zet here"; Flags: uninsdeletekey; Tasks: ctxmenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\zet"; ValueType: string; \
    ValueName: "Icon"; ValueData: """{app}\{#ExeName}"",0"; Tasks: ctxmenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\zet\command"; ValueType: string; \
    ValueName: ""; ValueData: """{app}\{#ExeName}"" --directory ""%V"""; Tasks: ctxmenu

Root: HKCU; Subkey: "Software\Classes\Directory\shell\zet"; ValueType: string; \
    ValueName: ""; ValueData: "Open zet here"; Flags: uninsdeletekey; Tasks: ctxmenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\zet"; ValueType: string; \
    ValueName: "Icon"; ValueData: """{app}\{#ExeName}"",0"; Tasks: ctxmenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\zet\command"; ValueType: string; \
    ValueName: ""; ValueData: """{app}\{#ExeName}"" --directory ""%V"""; Tasks: ctxmenu

[Code]
const
  { The user's own PATH lives here. The system PATH is HKLM and is not ours to touch. }
  EnvironmentKey = 'Environment';

{ --- PATH, installing --------------------------------------------------------- }

function NeedsPathEntry(const Dir: string): Boolean;
var
  Existing, Needle: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Existing) then
  begin
    { No user PATH at all yet, so there is nothing for this entry to duplicate. }
    Result := True;
    Exit;
  end;
  { Wrapped in separators on both sides so the first and last entries match too. }
  Needle := ';' + Uppercase(Dir) + ';';
  Result := Pos(Needle, ';' + Uppercase(Existing) + ';') = 0;
  if Result then
  begin
    { A trailing backslash on either side should not make it look absent. }
    Needle := ';' + Uppercase(Dir) + '\;';
    Result := Pos(Needle, ';' + Uppercase(Existing) + ';') = 0;
  end;
end;

procedure AddToUserPath(const Dir: string);
var
  Existing: string;
begin
  if not NeedsPathEntry(Dir) then
  begin
    Log(Format('"%s" is already in the user PATH.', [Dir]));
    Exit;
  end;

  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Existing) then
    Existing := '';

  { Built by hand rather than with a "{olddata};{app}" [Registry] entry. That entry
    expands to a leading ';' when the user has no PATH value yet, and a leading empty
    element in PATH means the current directory — which is how a terminal ends up
    running a program out of whatever folder it happens to be sitting in. }
  if Existing = '' then
    Existing := Dir
  else if Existing[Length(Existing)] = ';' then
    Existing := Existing + Dir
  else
    Existing := Existing + ';' + Dir;

  if RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Existing) then
    Log(Format('Added "%s" to the user PATH.', [Dir]))
  else
    Log(Format('Could not add "%s" to the user PATH.', [Dir]));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToUserPath(ExpandConstant('{app}'));
end;

{ --- PATH, uninstalling ------------------------------------------------------- }

procedure RemoveFromUserPath(const Dir: string);
var
  Paths, Trimmed, Needle: string;
  At: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Paths) then
  begin
    Log('There is no user PATH, so there is nothing to remove.');
    Exit;
  end;

  Trimmed := Dir;
  while (Length(Trimmed) > 3) and (Trimmed[Length(Trimmed)] = '\') do
    Delete(Trimmed, Length(Trimmed), 1);

  Needle := ';' + Uppercase(Trimmed) + ';';
  At := Pos(Needle, ';' + Uppercase(Paths) + ';');
  if At = 0 then
  begin
    Log(Format('"%s" is not in the user PATH, so there is nothing to remove.', [Trimmed]));
    Exit;
  end;

  { `At` points at the separator *before* the entry, in the wrapped string. When the
    entry is the very first element, At is 1 and that separator is the one this code
    added, not one that belongs to PATH — deleting the character before it would eat
    the first letter of whatever follows. Published snippets that always delete at
    At - 1 corrupt PATH in exactly that case. }
  if At > 1 then
    Delete(Paths, At - 1, Length(Trimmed) + 1)
  else
    Delete(Paths, 1, Length(Trimmed) + 1);

  { ExpandString, not String: writing a REG_EXPAND_SZ PATH as REG_SZ freezes every
    %VARIABLE% already in it. }
  if Paths = '' then
  begin
    if RegDeleteValue(HKCU, EnvironmentKey, 'Path') then
      Log('The user PATH is now empty, so the value was removed.');
  end
  else if RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Paths) then
    Log(Format('Removed "%s" from the user PATH.', [Trimmed]))
  else
    Log(Format('Could not remove "%s" from the user PATH.', [Trimmed]));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveFromUserPath(ExpandConstant('{app}'));
end;
