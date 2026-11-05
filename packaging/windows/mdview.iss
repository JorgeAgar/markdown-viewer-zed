#ifndef AppVersion
  #error AppVersion must be supplied by the release workflow
#endif
#ifndef SourceDir
  #error SourceDir must be supplied by the release workflow
#endif
[Setup]
AppId={{B6D670A2-91B1-4554-BE9D-2715A7F30680}
AppName=mdview
AppVersion={#AppVersion}
AppPublisher=mdview contributors
AppPublisherURL=https://github.com/JorgeAgar/markdown-viewer-zed
DefaultDirName={localappdata}\Programs\mdview
DefaultGroupName=mdview
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=dist
OutputBaseFilename=mdview-v{#AppVersion}-windows-x86_64-setup
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\mdview.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=yes

[Tasks]
Name: "fileassociation"; Description: "Add mdview to Open with for Markdown files"; Flags: unchecked
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourceDir}\mdview.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\docs\releases.md"; DestDir: "{app}"; DestName: "RELEASES.md"; Flags: ignoreversion

[Icons]
Name: "{group}\mdview"; Filename: "{app}\mdview.exe"
Name: "{autodesktop}\mdview"; Filename: "{app}\mdview.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Classes\mdview.Markdown"; ValueType: string; ValueData: "Markdown document"; Flags: uninsdeletekey; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\mdview.Markdown\DefaultIcon"; ValueType: string; ValueData: "{app}\mdview.exe,0"; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\mdview.Markdown\shell\open\command"; ValueType: string; ValueData: """{app}\mdview.exe"" ""%1"""; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: none; ValueName: "mdview.Markdown"; Flags: uninsdeletevalue; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: none; ValueName: "mdview.Markdown"; Flags: uninsdeletevalue; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\.mdx\OpenWithProgids"; ValueType: none; ValueName: "mdview.Markdown"; Flags: uninsdeletevalue; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\.mdown\OpenWithProgids"; ValueType: none; ValueName: "mdview.Markdown"; Flags: uninsdeletevalue; Tasks: fileassociation
Root: HKCU; Subkey: "Software\Classes\.mkd\OpenWithProgids"; ValueType: none; ValueName: "mdview.Markdown"; Flags: uninsdeletevalue; Tasks: fileassociation
