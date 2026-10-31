; Installateur de Bonk : raccourci du menu Démarrer et désinstallation, sans droits d'administrateur. Le disque,
; lui, demandera son autorisation au moment d'écrire ; l'interface ne tourne jamais en administrateur.
#define Version GetEnv("BONK_VERSION")
#define Arch GetEnv("BONK_ARCH")
#define Allowed GetEnv("BONK_ALLOWED")

[Setup]
AppId={{5C7C1D0E-7A3E-4C8B-9B2F-B0A1C0B0C0B1}
AppName=Bonk
AppVersion={#Version}
AppPublisher=FantasmaGlad
AppPublisherURL=https://github.com/FantasmaGlad/Bonk
DefaultDirName={autopf}\Bonk
DefaultGroupName=Bonk
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ChangesEnvironment=yes
OutputBaseFilename=bonk-{#Version}-windows-{#Arch}-setup
OutputDir=..\..\dist
SetupIconFile=..\..\crates\bonk\assets\bonk.ico
UninstallDisplayIcon={app}\bonk.exe
LicenseFile=..\..\LICENSE
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed={#Allowed}
ArchitecturesInstallIn64BitMode={#GetEnv("BONK_64")}

[Files]
Source: "{#GetEnv('BONK_EXE')}"; DestDir: "{app}"; DestName: "bonk.exe"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Bonk"; Filename: "{app}\bonk.exe"

[Run]
Filename: "{app}\bonk.exe"; Description: "Bonk"; Flags: nowait postinstall skipifsilent
