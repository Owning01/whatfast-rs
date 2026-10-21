; Whatfast - instalador de Windows.
;
; El paquete tiene los dos ejecutables: la interfaz nativa y el sidecar que
; habla con WhatsApp. Los dos tienen que quedar en la misma carpeta.
;
; Se compila con makensis (NSIS). El workflow de release lo arma en el
; runner de GitHub: choco install nsis && makensis installer.nsi

Unicode true

!include "MUI2.nsh"

!define APP_NAME "Whatfast"
!define APP_EXE "whatfast.exe"
!define BACKEND_EXE "whatfast-backend.exe"
!define APP_PUBLISHER "Owning01"
!define APP_VERSION "0.1.0"

Name "${APP_NAME}"
OutFile "whatfast-setup.exe"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
InstallDirRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "InstallLocation"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

; The icon of Whatfast: the one of the installer, the one of the app in the
; Explorador (the executable carries it) and the one of the shortcuts.
Icon "assets\icons\icon.ico"
!define APP_ICON "$INSTDIR\icons\icon.ico"

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey "FileDescription" "Cliente de escritorio de WhatsApp"
VIAddVersionKey "FileVersion" "${APP_VERSION}.0"
VIAddVersionKey "ProductVersion" "${APP_VERSION}.0"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

Section "Instalar ${APP_NAME}"
  SetOutPath "$INSTDIR"
  File "dist\whatfast.exe"
  File "dist\whatfast-backend.exe"
  SetOutPath "$INSTDIR\icons"
  File /r "dist\icons\*"

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "${APP_ICON}"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\Desinstalar.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "${APP_ICON}"

  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" "InstallLocation" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\whatfast.exe"
  Delete "$INSTDIR\whatfast-backend.exe"
  RMDir /r "$INSTDIR\icons"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\Desinstalar.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"

  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
SectionEnd
