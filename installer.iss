; Aura Inno Setup installer script

[Setup]
AppId={{0012DB8B-711D-4965-8935-F7E3E10A2D81}}
AppName=Aura
AppVersion=0.1.0
AppPublisher=JackSai
AppPublisherURL=https://github.com/Jack-Sai/aura
AppSupportURL=https://github.com/Jack-Sai/aura/issues
VersionInfoVersion=0.1.0
DefaultDirName={autopf}\Aura
DefaultGroupName=Aura
DisableProgramGroupPage=yes
LicenseFile=LICENSE
OutputDir=dist\installer
OutputBaseFilename=Aura_0.1.0_win_x64_Setup
SetupIconFile=src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\aura.exe
UninstallDisplayName=Aura
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "src-tauri\target\release\aura.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Aura"; Filename: "{app}\aura.exe"
Name: "{autodesktop}\Aura"; Filename: "{app}\aura.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\aura.exe"; Description: "Launch Aura"; Flags: nowait postinstall skipifsilent

[Code]
function WebView2Installed(): Boolean;
var
  pv: String;
begin
  Result := RegQueryStringValue(HKLM,
    'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'pv', pv) and (pv <> '');
  if not Result then
    Result := RegQueryStringValue(HKLM32,
      'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
      'pv', pv) and (pv <> '');
end;

function InitializeSetup(): Boolean;
begin
  if not WebView2Installed() then
    MsgBox('Microsoft Edge WebView2 Runtime was not detected. Aura may fail to start.' + #13#10 + #13#10 +
      'Install it from:' + #13#10 +
      'https://developer.microsoft.com/en-us/microsoft-edge/webview2/',
      mbInformation, MB_OK);
  Result := True;
end;
