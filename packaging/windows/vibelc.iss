; Windows installer, built with Inno Setup 6 on Windows:
;   iscc packaging\windows\vibelc.iss
; It packs the VibeLC folder that build-zip.sh stages in target\windows\VibeLC (or /DStage=DIR)
; into target\windows\vibelc-<version>-setup.exe. The version is read from vibelc.exe.

#ifndef Stage
  #define Stage SourcePath + "..\..\target\windows\VibeLC"
#endif
#ifndef Version
  #define Version GetStringFileInfo(Stage + "\vibelc.exe", "ProductVersion")
#endif
#define ProgId "VibeLC.Media"
; Video, audio and playlist files offered in "Open with" and in Windows' default apps.
#dim Exts[30] {".mp4", ".m4v", ".mkv", ".webm", ".avi", ".mov", ".wmv", ".asf", ".flv", ".mpg", \
  ".mpeg", ".vob", ".ts", ".m2ts", ".mts", ".3gp", ".ogv", ".mp3", ".m4a", ".aac", ".flac", ".ogg", \
  ".opus", ".wav", ".wma", ".mka", ".ac3", ".dts", ".m3u", ".m3u8"}

[Setup]
AppId={{6B0E3F2A-5C1D-4E8B-9A47-2F3C8D1E0B95}
AppName=VibeLC
AppVersion={#Version}
AppVerName=VibeLC {#Version}
AppPublisher=VibeLC contributors
AppPublisherURL=https://github.com/davsar89/VibeLC
VersionInfoVersion={#Version}
DefaultDirName={autopf}\VibeLC
DefaultGroupName=VibeLC
AllowNoIcons=yes
; For the current user without administrator rights, or for everyone.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile={#Stage}\LICENSE
SetupIconFile={#SourcePath}..\..\assets\icon\vibelc.ico
UninstallDisplayIcon={app}\vibelc.exe
ChangesAssociations=yes
; Over an earlier or the same version (same AppId), setup reuses its folder, install mode and
; tasks, and replaces the files in place; a running VibeLC is closed first, not restarted after.
CloseApplications=yes
RestartApplications=no
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir={#SourcePath}..\..\target\windows
OutputBaseFilename=vibelc-{#Version}-setup

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[CustomMessages]
en.Associate=Add VibeLC to "Open with" for video and audio files
fr.Associate=Ajouter VibeLC à « Ouvrir avec » pour les fichiers vidéo et audio
en.Integration=Windows integration:
fr.Integration=Intégration à Windows :

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "associate"; Description: "{cm:Associate}"; GroupDescription: "{cm:Integration}"

[Files]
Source: "{#Stage}\vibelc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\*.txt"; DestDir: "{app}"
Source: "{#Stage}\*.md"; DestDir: "{app}"
Source: "{#Stage}\LICENSE"; DestDir: "{app}"

[InstallDelete]
; Up to 1.0.1, libmpv was a DLL next to vibelc.exe; it is now linked in.
Type: files; Name: "{app}\libmpv-2.dll"
Type: files; Name: "{app}\vulkan-1.dll"
Type: files; Name: "{app}\libmpv-source.txt"
Type: files; Name: "{app}\VulkanRT-License.txt"

[Icons]
Name: "{group}\VibeLC"; Filename: "{app}\vibelc.exe"
Name: "{group}\{cm:UninstallProgram,VibeLC}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\VibeLC"; Filename: "{app}\vibelc.exe"; Tasks: desktopicon

[Registry]
; One program ID for every type; Windows then lists VibeLC under "Open with" and in Settings >
; Default apps, where the user picks it (Windows 10 and 11 let no installer take the default).
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueData: "Media file (VibeLC)"; Flags: uninsdeletekey; Tasks: associate
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueData: "{app}\vibelc.exe,0"; Tasks: associate
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueData: """{app}\vibelc.exe"" ""%1"""; Tasks: associate
Root: HKA; Subkey: "Software\Classes\Applications\vibelc.exe\shell\open\command"; ValueType: string; ValueData: """{app}\vibelc.exe"" ""%1"""; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\VibeLC\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "VibeLC"; Flags: uninsdeletekey; Tasks: associate
Root: HKA; Subkey: "Software\VibeLC\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Lightweight VLC-like video player"; Tasks: associate
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "VibeLC"; ValueData: "Software\VibeLC\Capabilities"; Flags: uninsdeletevalue; Tasks: associate
#sub AddExt
Root: HKA; Subkey: "Software\Classes\{#Exts[i]}\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\VibeLC\Capabilities\FileAssociations"; ValueType: string; ValueName: "{#Exts[i]}"; ValueData: "{#ProgId}"; Tasks: associate
#endsub
#define i
#for {i = 0; i < DimOf(Exts); i++} AddExt

[Run]
Filename: "{app}\vibelc.exe"; Description: "{cm:LaunchProgram,VibeLC}"; Flags: nowait postinstall skipifsilent
