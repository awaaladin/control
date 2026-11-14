; PULSE engine installer (Inno Setup 6). Built by .github/workflows/build-windows.yml:
;   ISCC.exe /DAppVersion=0.1.0 installer\pulse.iss   ->  installer\Output\PULSE-Setup.exe
; Per-user, no administrator rights. Everything it writes is under the install folder and HKCU.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef EngineExe
  #define EngineExe "..\target\release\rhcsd.exe"
#endif
#ifndef WebDist
  #define WebDist "..\web\dist"
#endif
#define SiteUrl "https://rhcs-pulse.vercel.app"

[Setup]
AppId={{D6F7CA16-EB8B-4A43-987E-B39453F37750}
AppName=PULSE
AppVersion={#AppVersion}
AppPublisher=rhcs
AppPublisherURL=https://github.com/awaaladin/control
DefaultDirName={localappdata}\PULSE
DefaultGroupName=PULSE
; The folder page stays visible so the engine can go on another drive (for example D:).
DisableDirPage=no
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
LicenseFile=..\LICENSE
InfoBeforeFile=NOTICE.txt
OutputDir=Output
OutputBaseFilename=PULSE-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=PULSE engine
CloseApplications=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "autostart"; Description: "Start PULSE when I sign in"; GroupDescription: "Engine:"

[Files]
Source: "{#EngineExe}"; DestDir: "{app}"; DestName: "rhcsd.exe"; Flags: ignoreversion
Source: "{#WebDist}\*"; DestDir: "{app}\web\dist"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "launch.vbs"; DestDir: "{app}"; Flags: ignoreversion
Source: "stop.cmd"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\PULSE\Start PULSE engine"; Filename: "{sys}\wscript.exe"; Parameters: """{app}\launch.vbs"" start"; WorkingDir: "{app}"
Name: "{userprograms}\PULSE\Stop PULSE engine"; Filename: "{app}\stop.cmd"; WorkingDir: "{app}"
Name: "{userprograms}\PULSE\Open PULSE"; Filename: "{#SiteUrl}/"

[Registry]
; pulse: link. It can only ever run launch.vbs, which accepts nothing but "start".
Root: HKCU; Subkey: "Software\Classes\pulse"; ValueType: string; ValueName: ""; ValueData: "URL:PULSE"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\pulse"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\pulse\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{sys}\wscript.exe"" ""{app}\launch.vbs"" ""%1"""
; Optional autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "PULSE"; ValueData: """{sys}\wscript.exe"" ""{app}\launch.vbs"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
; Start the engine now (hidden).
Filename: "{sys}\wscript.exe"; Parameters: """{app}\launch.vbs"" start"; WorkingDir: "{app}"; Flags: nowait runhidden
; Pairing link: the part after # never leaves the browser.
Filename: "{code:PairUrl}"; Description: "Open PULSE in my browser and connect it to the engine"; Flags: shellexec postinstall skipifsilent nowait

[UninstallRun]
Filename: "{app}\stop.cmd"; Flags: runhidden waituntilterminated; RunOnceId: "StopEngine"

[UninstallDelete]
Type: filesandordirs; Name: "{app}\web"
Type: filesandordirs; Name: "{app}\data"
Type: files; Name: "{app}\.env"
Type: files; Name: "{app}\rhcsd.log"
Type: files; Name: "{app}\rhcsd.log.old"
Type: dirifempty; Name: "{app}"

[Code]
var
  Token: String;

function ReadTokenFromEnv(const EnvFile: String): String;
var
  Lines: TArrayOfString;
  I: Integer;
  L: String;
begin
  Result := '';
  if LoadStringsFromFile(EnvFile, Lines) then
    for I := 0 to GetArrayLength(Lines) - 1 do
    begin
      L := Trim(Lines[I]);
      if Pos('RHCSD_TOKEN=', L) = 1 then
      begin
        Result := Trim(Copy(L, Length('RHCSD_TOKEN=') + 1, MaxInt));
        Exit;
      end;
    end;
end;

{ 48 hex characters from the operating system's random generator. }
function MakeToken: String;
var
  Tmp, Ps: String;
  Code: Integer;
  Lines: TArrayOfString;
begin
  Result := '';
  Tmp := ExpandConstant('{tmp}\tok.txt');
  Ps := '$b = New-Object byte[] 24; ' +
        '$g = [System.Security.Cryptography.RandomNumberGenerator]::Create(); $g.GetBytes($b); ' +
        '(($b | ForEach-Object { $_.ToString(''x2'') }) -join '''') | Set-Content -NoNewline -Encoding ASCII -LiteralPath $env:TOKFILE';
  { The output path travels in an environment variable, so no quoting problems. }
  SetEnvironmentVariable('TOKFILE', Tmp);
  if Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
          '-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "' + Ps + '"',
          '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
    if LoadStringsFromFile(Tmp, Lines) and (GetArrayLength(Lines) > 0) then
      Result := Trim(Lines[0]);
  if Length(Result) < 32 then
    RaiseException('Could not create a random access token (PowerShell failed). Nothing was configured.');
end;

procedure StopOldEngine;
var
  Code: Integer;
begin
  if FileExists(ExpandConstant('{app}\stop.cmd')) then
    Exec(ExpandConstant('{app}\stop.cmd'), '', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  StopOldEngine; { an upgrade must not find rhcsd.exe locked }
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  EnvFile: String;
begin
  if CurStep = ssPostInstall then
  begin
    EnvFile := ExpandConstant('{app}\.env');
    if FileExists(EnvFile) then
      Token := ReadTokenFromEnv(EnvFile) { upgrade: keep the settings and the token }
    else
    begin
      Token := MakeToken;
      SaveStringToFile(EnvFile,
        '# PULSE engine settings (created by the installer). Add ROUTER_IP etc. here if you need them.' + #13#10 +
        'RHCSD_BIND=127.0.0.1:7878' + #13#10 +
        'RHCSD_TOKEN=' + Token + #13#10, False);
    end;
  end;
end;

function PairUrl(Param: String): String;
begin
  Result := '{#SiteUrl}/#hub=http%3A%2F%2F127.0.0.1%3A7878';
  if Token <> '' then
    Result := Result + '&token=' + Token;
end;
