; The installer (NSIS), for one processor. It is not run by hand: `win-installer` (`just package`) gives it everything through -D definitions
; (the product's name from the branding, the programs, the driver's installer, the version...). Texts are in English and French; the installer
; speaks the language of Windows.
;
; What it does:
;   first installation : welcome, licence, options, files, end
;   already installed  : a page that offers to update (or repair) or to remove
;   silent             : /S, with /INSTALL_AUTOSTART=0|1 /INSTALL_DRIVER=0|1 /INSTALL_CLI=0|1 /INSTALL_PATH=0|1 /INSTALL_DESKTOP=0|1 /D=folder
;                        (an installation that finds the program already there updates or repairs it); remove: "<folder>\uninstall.exe" /S

Unicode true
SetCompressor /SOLID lzma

!macro Need NAME
  !ifndef ${NAME}
    !error "${NAME} is not defined: this script is run by win-installer, which gives it its definitions"
  !endif
!macroend
!insertmacro Need PRODUCT_NAME
!insertmacro Need SLUG
!insertmacro Need VERSION
!insertmacro Need VERSION_QUAD
!insertmacro Need ARCH
!insertmacro Need PROGRAMS_DIR
!insertmacro Need DAEMON_EXE
!insertmacro Need CLI_EXE
!insertmacro Need GUI_EXE
!insertmacro Need ICON_FILE
!insertmacro Need LICENSE_FILE
!insertmacro Need NOTICES_FILE
!insertmacro Need DATA_DIR_NAME
!insertmacro Need APP_ID
!insertmacro Need OUT_FILE

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "nsDialogs.nsh"
!include "WordFunc.nsh"

!insertmacro GetParameters
!insertmacro GetOptions
!insertmacro un.GetParameters
!insertmacro un.GetOptions
!insertmacro VersionCompare
!insertmacro GetSize

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${SLUG}"
!define SETTINGS_KEY "Software\${SLUG}"
; where the driver's own installer says it installed itself
!define DRIVER_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\{199505b0-b93d-4521-a8c7-897818e0205a}_is1"

Name "${PRODUCT_NAME}"
OutFile "${OUT_FILE}"
InstallDir "$PROGRAMFILES64\${PRODUCT_NAME}"
RequestExecutionLevel admin
ManifestDPIAware true
BrandingText "${PRODUCT_NAME} ${VERSION}"
Icon "${ICON_FILE}"
UninstallIcon "${ICON_FILE}"

VIProductVersion "${VERSION_QUAD}"
VIAddVersionKey "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${PRODUCT_NAME} installer"
VIAddVersionKey "LegalCopyright" "${PRODUCT_NAME}"

; ------------------------------------------------------------------------------------------------------------------ state
Var Installed        ; 1 when this computer already has the program
Var InstalledVersion
Var Choice           ; what the person wants of an installation that exists: "update" or "remove"
Var OptAutostart
Var OptDriver
Var OptCli
Var OptPath
Var OptDesktop
Var DriverPresent    ; 1 when the driver is already on this computer
Var Dialog
Var Radio1
Var Radio2
Var BoxAutostart
Var BoxDriver
Var BoxCli
Var BoxPath
Var BoxDesktop
Var DeleteData       ; the uninstaller: 1 when the person's data goes too
Var BoxDelete

; ------------------------------------------------------------------------------------------------------------------ pages
!define MUI_ICON "${ICON_FILE}"
!define MUI_UNICON "${ICON_FILE}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "$(LAUNCH)"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchWindow

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfInstalled
!insertmacro MUI_PAGE_WELCOME
Page custom MaintenancePage MaintenanceLeave
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfInstalled
!insertmacro MUI_PAGE_LICENSE "${LICENSE_FILE}"
Page custom OptionsPage OptionsLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
UninstPage custom un.DataPage un.DataLeave
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "French"

; ------------------------------------------------------------------------------------------------------------------ texts
LangString LAUNCH ${LANG_ENGLISH} "Open ${PRODUCT_NAME} now"
LangString LAUNCH ${LANG_FRENCH} "Ouvrir ${PRODUCT_NAME} maintenant"
LangString WRONG_PROCESSOR ${LANG_ENGLISH} "This installer is for ${ARCH} computers. Use the one that matches this computer's processor."
LangString WRONG_PROCESSOR ${LANG_FRENCH} "Cet installateur est pour les ordinateurs ${ARCH}. Utilisez celui qui correspond au processeur de cet ordinateur."
LangString NEWER_INSTALLED ${LANG_ENGLISH} "A newer version of ${PRODUCT_NAME} ($InstalledVersion) is already installed. Remove it first to install this one."
LangString NEWER_INSTALLED ${LANG_FRENCH} "Une version plus récente de ${PRODUCT_NAME} ($InstalledVersion) est déjà installée. Désinstallez-la d'abord pour installer celle-ci."

LangString MAINT_TITLE ${LANG_ENGLISH} "${PRODUCT_NAME} is already installed"
LangString MAINT_TITLE ${LANG_FRENCH} "${PRODUCT_NAME} est déjà installé"
LangString MAINT_SUBTITLE ${LANG_ENGLISH} "Version $InstalledVersion. Choose what to do."
LangString MAINT_SUBTITLE ${LANG_FRENCH} "Version $InstalledVersion. Choisissez ce qu'il faut faire."
LangString MAINT_UPDATE ${LANG_ENGLISH} "Update to version ${VERSION}"
LangString MAINT_UPDATE ${LANG_FRENCH} "Mettre à jour vers la version ${VERSION}"
LangString MAINT_REPAIR ${LANG_ENGLISH} "Repair (put the program's files back)"
LangString MAINT_REPAIR ${LANG_FRENCH} "Réparer (remettre les fichiers du programme)"
LangString MAINT_REMOVE ${LANG_ENGLISH} "Remove ${PRODUCT_NAME} (your settings, graphs and sounds are kept)"
LangString MAINT_REMOVE ${LANG_FRENCH} "Désinstaller ${PRODUCT_NAME} (vos réglages, graphes et sons sont conservés)"

LangString OPT_TITLE ${LANG_ENGLISH} "Options"
LangString OPT_TITLE ${LANG_FRENCH} "Options"
LangString OPT_SUBTITLE ${LANG_ENGLISH} "Choose what is set up with ${PRODUCT_NAME}."
LangString OPT_SUBTITLE ${LANG_FRENCH} "Choisissez ce qui est mis en place avec ${PRODUCT_NAME}."
LangString OPT_AUTOSTART ${LANG_ENGLISH} "Start ${PRODUCT_NAME} with Windows"
LangString OPT_AUTOSTART ${LANG_FRENCH} "Lancer ${PRODUCT_NAME} avec Windows"
LangString OPT_DRIVER ${LANG_ENGLISH} "Install the driver of the virtual microphones and speakers"
LangString OPT_DRIVER ${LANG_FRENCH} "Installer le pilote des microphones et haut-parleurs virtuels"
LangString OPT_DRIVER_ABOUT ${LANG_ENGLISH} "A driver made by someone else (usbip-win2). Your USB devices disconnect for a moment while it is installed. Without it, ${PRODUCT_NAME} still works with your real sound devices. Removing ${PRODUCT_NAME} does not remove it."
LangString OPT_DRIVER_ABOUT ${LANG_FRENCH} "Un pilote fait par quelqu'un d'autre (usbip-win2). Vos périphériques USB se déconnectent un instant pendant son installation. Sans lui, ${PRODUCT_NAME} fonctionne quand même avec vos vrais périphériques de son. Désinstaller ${PRODUCT_NAME} ne le retire pas."
LangString OPT_DRIVER_PRESENT ${LANG_ENGLISH} "The driver is already installed on this computer."
LangString OPT_DRIVER_PRESENT ${LANG_FRENCH} "Le pilote est déjà installé sur cet ordinateur."
LangString OPT_CLI ${LANG_ENGLISH} "Install the command line (${CLI_EXE})"
LangString OPT_CLI ${LANG_FRENCH} "Installer la ligne de commande (${CLI_EXE})"
LangString OPT_PATH ${LANG_ENGLISH} "Add it to the PATH, to type its name in any terminal"
LangString OPT_PATH ${LANG_FRENCH} "L'ajouter au PATH, pour taper son nom dans n'importe quel terminal"
LangString OPT_DESKTOP ${LANG_ENGLISH} "Put a shortcut on the desktop"
LangString OPT_DESKTOP ${LANG_FRENCH} "Mettre un raccourci sur le bureau"

LangString UN_DATA_TITLE ${LANG_ENGLISH} "Your data"
LangString UN_DATA_TITLE ${LANG_FRENCH} "Vos données"
LangString UN_DATA_SUBTITLE ${LANG_ENGLISH} "Choose what stays on this computer."
LangString UN_DATA_SUBTITLE ${LANG_FRENCH} "Choisissez ce qui reste sur cet ordinateur."
LangString UN_DATA_BOX ${LANG_ENGLISH} "Also delete my data and everything ${PRODUCT_NAME} set up on this computer"
LangString UN_DATA_BOX ${LANG_FRENCH} "Supprimer aussi mes données et tout ce que ${PRODUCT_NAME} a mis en place sur cet ordinateur"
LangString UN_DATA_ABOUT ${LANG_ENGLISH} "Settings, graphs, soundboards, sounds, logs, downloaded modules, the permanent virtual devices and the remembered devices. This cannot be undone. If you leave this box empty, they are kept and found again if you install ${PRODUCT_NAME} later."
LangString UN_DATA_ABOUT ${LANG_FRENCH} "Réglages, graphes, soundboards, sons, journaux, modules téléchargés, périphériques virtuels permanents et périphériques mémorisés. C'est définitif. Si vous laissez la case vide, ils sont conservés et retrouvés si vous réinstallez ${PRODUCT_NAME} plus tard."

LangString STOPPING ${LANG_ENGLISH} "Stopping ${PRODUCT_NAME}..."
LangString STOPPING ${LANG_FRENCH} "Arrêt de ${PRODUCT_NAME}..."
LangString DRIVER_INSTALLING ${LANG_ENGLISH} "Installing the driver (this takes a moment)..."
LangString DRIVER_INSTALLING ${LANG_FRENCH} "Installation du pilote (cela prend un moment)..."
LangString DRIVER_FAILED ${LANG_ENGLISH} "The driver could not be installed. ${PRODUCT_NAME} works without it; it says what is missing when the virtual devices are needed."
LangString DRIVER_FAILED ${LANG_FRENCH} "Le pilote n'a pas pu être installé. ${PRODUCT_NAME} fonctionne sans lui ; il dit ce qui manque quand les périphériques virtuels sont nécessaires."

; ------------------------------------------------------------------------------------------------------------------ start
Function .onInit
  SetRegView 64
  ; the right processor
  ReadEnvStr $0 PROCESSOR_ARCHITEW6432
  ${If} $0 == ""
    ReadEnvStr $0 PROCESSOR_ARCHITECTURE
  ${EndIf}
  !if "${ARCH}" == "arm64"
    StrCmp $0 "ARM64" processor_ok
  !else
    StrCmp $0 "AMD64" processor_ok
  !endif
  MessageBox MB_OK|MB_ICONSTOP "$(WRONG_PROCESSOR)" /SD IDOK
  SetErrorLevel 4
  Abort
processor_ok:

  ; the choices of the first installation, then what the command line says
  StrCpy $OptAutostart 1
  StrCpy $OptDriver 1
  StrCpy $OptCli 1
  StrCpy $OptPath 1
  StrCpy $OptDesktop 0
  StrCpy $Choice "update"

  ReadRegStr $InstalledVersion HKLM "${UNINSTALL_KEY}" "DisplayVersion"
  ReadRegStr $0 HKLM "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $InstalledVersion != ""
  ${AndIf} $0 != ""
    StrCpy $Installed 1
    StrCpy $INSTDIR $0
    ; an update or a repair keeps what was chosen the first time
    ClearErrors
    ReadRegDWORD $0 HKLM "${SETTINGS_KEY}" "Cli"
    ${IfNot} ${Errors}
      StrCpy $OptCli $0
    ${EndIf}
    ClearErrors
    ReadRegDWORD $0 HKLM "${SETTINGS_KEY}" "Path"
    ${IfNot} ${Errors}
      StrCpy $OptPath $0
    ${EndIf}
    ClearErrors
    ReadRegDWORD $0 HKLM "${SETTINGS_KEY}" "Desktop"
    ${IfNot} ${Errors}
      StrCpy $OptDesktop $0
    ${EndIf}
  ${Else}
    StrCpy $Installed 0
  ${EndIf}

  ${GetParameters} $R0
  ${GetOptions} $R0 "/INSTALL_AUTOSTART=" $R1
  ${IfNot} ${Errors}
    StrCpy $OptAutostart $R1
  ${EndIf}
  ${GetOptions} $R0 "/INSTALL_DRIVER=" $R1
  ${IfNot} ${Errors}
    StrCpy $OptDriver $R1
  ${EndIf}
  ${GetOptions} $R0 "/INSTALL_CLI=" $R1
  ${IfNot} ${Errors}
    StrCpy $OptCli $R1
  ${EndIf}
  ${GetOptions} $R0 "/INSTALL_PATH=" $R1
  ${IfNot} ${Errors}
    StrCpy $OptPath $R1
  ${EndIf}
  ${GetOptions} $R0 "/INSTALL_DESKTOP=" $R1
  ${IfNot} ${Errors}
    StrCpy $OptDesktop $R1
  ${EndIf}

  ReadRegStr $0 HKLM "${DRIVER_KEY}" "InstallLocation"
  ${If} $0 == ""
    StrCpy $DriverPresent 0
  ${Else}
    StrCpy $DriverPresent 1
  ${EndIf}

  ; an older version than the one that is installed is not put over it
  ${If} $Installed == 1
    ${VersionCompare} "$InstalledVersion" "${VERSION}" $0
    ${If} $0 == 1
      ${If} ${Silent}
        SetErrorLevel 5
        Abort
      ${EndIf}
    ${EndIf}
  ${EndIf}
FunctionEnd

Function SkipIfInstalled
  ${If} $Installed == 1
    Abort
  ${EndIf}
FunctionEnd

Function LaunchWindow
  ; the window is opened as the person, not with the rights of the installer
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\${GUI_EXE}"'
FunctionEnd

; ------------------------------------------------------------------------------------------------------------------ the page of an existing installation
Function MaintenancePage
  ${If} $Installed == 0
    Abort
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "$(MAINT_TITLE)" "$(MAINT_SUBTITLE)"
  nsDialogs::Create 1018
  Pop $Dialog
  ${If} $Dialog == error
    Abort
  ${EndIf}
  ${VersionCompare} "$InstalledVersion" "${VERSION}" $R0
  ${If} $R0 == 1
    ; a newer version is there: it can only be removed
    ${NSD_CreateLabel} 0 0 100% 24u "$(NEWER_INSTALLED)"
    Pop $0
  ${EndIf}
  ${If} $R0 == 0
    ${NSD_CreateRadioButton} 10u 30u 95% 12u "$(MAINT_REPAIR)"
  ${Else}
    ${NSD_CreateRadioButton} 10u 30u 95% 12u "$(MAINT_UPDATE)"
  ${EndIf}
  Pop $Radio1
  ${NSD_CreateRadioButton} 10u 50u 95% 12u "$(MAINT_REMOVE)"
  Pop $Radio2
  ${If} $R0 == 1
    EnableWindow $Radio1 0
    ${NSD_Check} $Radio2
  ${Else}
    ${NSD_Check} $Radio1
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function MaintenanceLeave
  ${NSD_GetState} $Radio2 $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $Choice "remove"
    ; the removal is the uninstaller's: it runs in place and is waited for
    ExecWait '"$INSTDIR\uninstall.exe" _?=$INSTDIR'
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"
    Quit
  ${EndIf}
FunctionEnd

; ------------------------------------------------------------------------------------------------------------------ the options page
Function OptionsPage
  ${If} $Installed == 1
    Abort
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "$(OPT_TITLE)" "$(OPT_SUBTITLE)"
  nsDialogs::Create 1018
  Pop $Dialog
  ${If} $Dialog == error
    Abort
  ${EndIf}

  ${NSD_CreateCheckbox} 0 0 100% 12u "$(OPT_AUTOSTART)"
  Pop $BoxAutostart
  ${If} $OptAutostart == 1
    ${NSD_Check} $BoxAutostart
  ${EndIf}

  ${NSD_CreateCheckbox} 0 20u 100% 12u "$(OPT_DRIVER)"
  Pop $BoxDriver
  !ifdef DRIVER_FILE
    ${If} $DriverPresent == 1
      EnableWindow $BoxDriver 0
      ${NSD_CreateLabel} 12u 33u 95% 20u "$(OPT_DRIVER_PRESENT)"
      Pop $0
    ${Else}
      ${If} $OptDriver == 1
        ${NSD_Check} $BoxDriver
      ${EndIf}
      ${NSD_CreateLabel} 12u 33u 95% 32u "$(OPT_DRIVER_ABOUT)"
      Pop $0
    ${EndIf}
  !else
    EnableWindow $BoxDriver 0
  !endif

  ${NSD_CreateCheckbox} 0 72u 100% 12u "$(OPT_CLI)"
  Pop $BoxCli
  ${If} $OptCli == 1
    ${NSD_Check} $BoxCli
  ${EndIf}
  ${NSD_OnClick} $BoxCli CliToggled

  ${NSD_CreateCheckbox} 12u 88u 95% 12u "$(OPT_PATH)"
  Pop $BoxPath
  ${If} $OptPath == 1
    ${NSD_Check} $BoxPath
  ${EndIf}
  ${If} $OptCli != 1
    EnableWindow $BoxPath 0
  ${EndIf}

  ${NSD_CreateCheckbox} 0 108u 100% 12u "$(OPT_DESKTOP)"
  Pop $BoxDesktop
  ${If} $OptDesktop == 1
    ${NSD_Check} $BoxDesktop
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function CliToggled
  ${NSD_GetState} $BoxCli $0
  ${If} $0 == ${BST_CHECKED}
    EnableWindow $BoxPath 1
  ${Else}
    EnableWindow $BoxPath 0
  ${EndIf}
FunctionEnd

Function OptionsLeave
  ${NSD_GetState} $BoxAutostart $OptAutostart
  ${NSD_GetState} $BoxDriver $OptDriver
  ${NSD_GetState} $BoxCli $OptCli
  ${NSD_GetState} $BoxPath $OptPath
  ${NSD_GetState} $BoxDesktop $OptDesktop
  ; a box that is checked reads 1, one that is not reads 0
FunctionEnd

; ------------------------------------------------------------------------------------------------------------------ stopping what runs
; The daemon is asked to stop (it takes its virtual devices down in order), then it is waited for; the window is closed.
!macro StopPrograms DIR
  DetailPrint "$(STOPPING)"
  IfFileExists "${DIR}\${DAEMON_EXE}" 0 +2
    nsExec::Exec '"${DIR}\${DAEMON_EXE}" --stop'
  Pop $0
  nsExec::Exec '"$SYSDIR\taskkill.exe" /IM "${GUI_EXE}"'
  Pop $0
  StrCpy $1 0
  ${Do}
    nsExec::Exec '"$SYSDIR\cmd.exe" /C tasklist /FI "IMAGENAME eq ${DAEMON_EXE}" /NH | find /I "${DAEMON_EXE}"'
    Pop $0
    ${If} $0 != 0
      ${Break}
    ${EndIf}
    IntOp $1 $1 + 1
    ${If} $1 > 12
      ; it does not stop by itself: it is made to
      nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM "${DAEMON_EXE}"'
      Pop $0
      ${Break}
    ${EndIf}
    Sleep 500
  ${Loop}
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM "${GUI_EXE}"'
  Pop $0
  Sleep 300
!macroend

; ------------------------------------------------------------------------------------------------------------------ install
Section "Install"
  SetRegView 64
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  !insertmacro StopPrograms "$INSTDIR"

  File "${PROGRAMS_DIR}\${DAEMON_EXE}"
  File "${PROGRAMS_DIR}\${GUI_EXE}"
  ${If} $OptCli == 1
    File "${PROGRAMS_DIR}\${CLI_EXE}"
  ${Else}
    Delete "$INSTDIR\${CLI_EXE}"
  ${EndIf}
  File "/oname=LICENSE.txt" "${LICENSE_FILE}"
  File "${NOTICES_FILE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; shortcuts
  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\${GUI_EXE}" "" "$INSTDIR\${GUI_EXE}" 0
  ${If} $OptDesktop == 1
    CreateShortcut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\${GUI_EXE}" "" "$INSTDIR\${GUI_EXE}" 0
  ${Else}
    Delete "$DESKTOP\${PRODUCT_NAME}.lnk"
  ${EndIf}

  ; the PATH of the computer: done by the program itself (it keeps the value's kind and length)
  ${If} $OptCli == 1
  ${AndIf} $OptPath == 1
    nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --set-path on'
  ${Else}
    nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --set-path off'
  ${EndIf}
  Pop $0

  ; what the first installation chose is remembered for the updates, and the entry of Windows' list of programs
  WriteRegDWORD HKLM "${SETTINGS_KEY}" "Cli" $OptCli
  WriteRegDWORD HKLM "${SETTINGS_KEY}" "Path" $OptPath
  WriteRegDWORD HKLM "${SETTINGS_KEY}" "Desktop" $OptDesktop
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${GUI_EXE}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" $0

  ; only the first installation sets up what the person chose: an update leaves their settings as they are
  ${If} $Installed == 0
    ${If} $OptAutostart == 1
      nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --set-start-at-logon on'
      Pop $0
    ${EndIf}
    !ifdef DRIVER_FILE
      ${If} $OptDriver == 1
      ${AndIf} $DriverPresent == 0
        DetailPrint "$(DRIVER_INSTALLING)"
        InitPluginsDir
        File "/oname=$PLUGINSDIR\driver.exe" "${DRIVER_FILE}"
        ExecWait '"$PLUGINSDIR\driver.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-' $0
        ${If} $0 != 0
          DetailPrint "$(DRIVER_FAILED) ($0)"
          MessageBox MB_OK|MB_ICONEXCLAMATION "$(DRIVER_FAILED)" /SD IDOK
        ${EndIf}
      ${EndIf}
    !endif
  ${EndIf}
SectionEnd

; ------------------------------------------------------------------------------------------------------------------ remove
Function un.onInit
  SetRegView 64
  StrCpy $DeleteData 0
  ${un.GetParameters} $R0
  ${un.GetOptions} $R0 "/DELETE_DATA=" $R1
  ${IfNot} ${Errors}
    StrCpy $DeleteData $R1
  ${EndIf}
FunctionEnd

Function un.DataPage
  !insertmacro MUI_HEADER_TEXT "$(UN_DATA_TITLE)" "$(UN_DATA_SUBTITLE)"
  nsDialogs::Create 1018
  Pop $Dialog
  ${If} $Dialog == error
    Abort
  ${EndIf}
  ${NSD_CreateCheckbox} 0 0 100% 24u "$(UN_DATA_BOX)"
  Pop $BoxDelete
  ${If} $DeleteData == 1
    ${NSD_Check} $BoxDelete
  ${EndIf}
  ${NSD_CreateLabel} 12u 28u 95% 50u "$(UN_DATA_ABOUT)"
  Pop $0
  nsDialogs::Show
FunctionEnd

Function un.DataLeave
  ${NSD_GetState} $BoxDelete $DeleteData
FunctionEnd

Section "Uninstall"
  SetRegView 64
  SetShellVarContext all
  !insertmacro StopPrograms "$INSTDIR"

  ; what was set up outside of the folder goes away first, while the program that knows how is still there
  nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --set-start-at-logon off'
  Pop $0
  nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --set-path off'
  Pop $0

  ${If} $DeleteData == 1
    ; what the person made, and what the system keeps for the program: the driver forgets the permanent devices, then the devices that are gone
    ; are forgotten (with an empty data folder of its own, so that none is kept for a saved graph), then the folders
    nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --forget-permanent-devices'
    Pop $0
    RMDir /r "$TEMP\${SLUG}-forget"
    nsExec::Exec '"$INSTDIR\${DAEMON_EXE}" --data-dir "$TEMP\${SLUG}-forget" --clean-devices'
    Pop $0
    RMDir /r "$TEMP\${SLUG}-forget"
    SetShellVarContext current
    RMDir /r "$APPDATA\${DATA_DIR_NAME}"
    RMDir /r "$LOCALAPPDATA\${APP_ID}"
    SetShellVarContext all
  ${EndIf}

  Delete "$INSTDIR\${DAEMON_EXE}"
  Delete "$INSTDIR\${GUI_EXE}"
  Delete "$INSTDIR\${CLI_EXE}"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  DeleteRegKey HKLM "${UNINSTALL_KEY}"
  DeleteRegKey HKLM "${SETTINGS_KEY}"
  ; without the box, the person's settings, graphs and sounds (in their own folder) stay; the driver always stays: other programs may use it
SectionEnd

; The programs and the uninstaller are signed when a certificate is given (see the builder): the signing program is run on each.
!ifdef SIGN_COMMAND
  !finalize '${SIGN_COMMAND}'
  !uninstfinalize '${SIGN_COMMAND}'
!endif
