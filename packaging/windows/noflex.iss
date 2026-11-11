; SPDX-License-Identifier: AGPL-3.0-only
;
; The Windows installer for a server bundle, compiled with Inno Setup by
; packaging/windows/installer.sh. It puts the bundle under Program Files and
; registers the service, which keeps its config, data and logs in
; %ProgramData%\NoFlex. Uninstalling leaves those.

#ifndef Version
  #error Pass /DVersion=<version>
#endif
#ifndef Bundle
  #error Pass /DBundle=<bundle directory>
#endif

[Setup]
; Never change: Windows matches upgrades to installs by it.
AppId={{C78A8FAF-8AC5-485E-BF74-E220849C09EB}
AppName=NoFlex
AppVersion={#Version}
AppVerName=NoFlex {#Version}
AppPublisher=NoFlex
AppPublisherURL=https://noflex.dev
AppSupportURL=https://github.com/NoFlexMedia/noflex/discussions
DefaultDirName={autopf}\NoFlex
DisableProgramGroupPage=yes
LicenseFile={#Bundle}\LICENSE
OutputBaseFilename=noflex-{#Version}-server-windows-x64-setup
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
PrivilegesRequired=admin
UninstallDisplayName=NoFlex media server
UninstallDisplayIcon={app}\noflex-server.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#Bundle}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[INI]
Filename: "{autoprograms}\NoFlex.url"; Section: "InternetShortcut"; Key: "URL"; String: "http://localhost:28793"

[UninstallDelete]
Type: files; Name: "{autoprograms}\NoFlex.url"

[Run]
; The database holds each user's keys to the servers they've linked, and
; ProgramData is readable by every account, so only SYSTEM and Administrators
; keep access.
Filename: "{sys}\icacls.exe"; Parameters: """{commonappdata}\NoFlex"" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F"; Flags: runhidden
Filename: "{app}\noflex-server.exe"; Parameters: "service install"; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=NoFlex"; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=NoFlex dir=in action=allow protocol=TCP localport=28793 program=""{app}\noflex-server.exe"""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "start NoFlex"; Flags: runhidden
Filename: "http://localhost:28793"; Description: "Open NoFlex"; Flags: postinstall shellexec nowait skipifsilent

[UninstallRun]
Filename: "{app}\noflex-server.exe"; Parameters: "service uninstall"; Flags: runhidden; RunOnceId: "Service"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=NoFlex"; Flags: runhidden; RunOnceId: "Firewall"

[Code]
// An upgrade replaces files the running service holds open, so stop it and
// let [Run] register the new one.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  if FileExists(ExpandConstant('{app}\noflex-server.exe')) then
    Exec(ExpandConstant('{app}\noflex-server.exe'), 'service uninstall', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

// The service's config, written once: an upgrade keeps whatever is there.
procedure WriteConfig;
var
  Dir, Config, NL: String;
begin
  Dir := ExpandConstant('{commonappdata}\NoFlex');
  Config := Dir + '\config.toml';
  if FileExists(Config) then
    Exit;
  NL := Chr(13) + Chr(10);
  ForceDirectories(Dir);
  SaveStringToFile(Config,
    '# NoFlex as a Windows service. Every setting is in docs/CONFIGURATION.md:' + NL +
    '# https://github.com/NoFlexMedia/noflex/blob/main/docs/CONFIGURATION.md' + NL + NL +
    'bind = "0.0.0.0:28793"' + NL +
    'data_dir = ''' + Dir + '\data''' + NL +
    'cache_dir = ''' + Dir + '\cache''' + NL + NL +
    '[log]' + NL +
    'file = ''' + Dir + '\logs\noflex.log''' + NL,
    False);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    WriteConfig;
end;
