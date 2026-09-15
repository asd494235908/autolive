Unicode true
ManifestSupportedOS all
ManifestDPIAware true
RequestExecutionLevel admin

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "WinVer.nsh"
!include "x64.nsh"

!ifndef APP_PACKAGE_ROOT
  !error "APP_PACKAGE_ROOT is required"
!endif
!ifndef DOTNET_RUNTIME_PATH
  !error "DOTNET_RUNTIME_PATH is required"
!endif
!ifndef OUTPUT_PATH
  !error "OUTPUT_PATH is required"
!endif
!ifndef PACKAGE_VERSION
  !error "PACKAGE_VERSION is required"
!endif
!ifndef PRODUCT_VERSION
  !error "PRODUCT_VERSION is required"
!endif
!ifndef APP_ICON
  !error "APP_ICON is required"
!endif
!ifndef CONTROL_PLANE_PROFILE
  !error "CONTROL_PLANE_PROFILE is required"
!endif

!define PRODUCT_NAME "GpAutoLive"
!define PRODUCT_PUBLISHER "Gepin Technology"
!define PRODUCT_REG_KEY "Software\GpAutoLive\CSharp.Windows"
!define UNINSTALL_REG_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\GpAutoLive.CSharp.Windows"
!define APP_EXE "$INSTDIR\app\GpAutoLive.exe"
!define DOTNET_RUNTIME_FILE "windowsdesktop-runtime-10.0.11-win-x64.exe"

Name "${PRODUCT_NAME}"
OutFile "${OUTPUT_PATH}"
InstallDir "$PROGRAMFILES64\GpAutoLive"
InstallDirRegKey HKLM "${PRODUCT_REG_KEY}" "InstallDir"
SetCompressor /SOLID lzma
SetCompressorDictSize 64
CRCCheck force
BrandingText "GpAutoLive ${PACKAGE_VERSION}"
VIProductVersion "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=2052 "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey /LANG=2052 "CompanyName" "${PRODUCT_PUBLISHER}"
VIAddVersionKey /LANG=2052 "FileDescription" "GpAutoLive Windows 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=2052 "ProductVersion" "${PACKAGE_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "Copyright 2026 Gepin Technology"

!define MUI_ABORTWARNING
!define MUI_ICON "${APP_ICON}"
!define MUI_UNICON "${APP_ICON}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Function EnsureSupportedSystem
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP|MB_OK "GpAutoLive 仅支持 64 位 Windows。"
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP|MB_OK "GpAutoLive 需要 Windows 10 2004 或更高版本。"
    Abort
  ${EndIf}
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentBuildNumber"
  IntCmp $0 19041 supported_system unsupported_system supported_system
  unsupported_system:
    MessageBox MB_ICONSTOP|MB_OK "GpAutoLive 需要 Windows 10 2004（内部版本 19041）或更高版本。"
    Abort
  supported_system:
FunctionEnd

Function EnsureAppStopped
  IfFileExists "$LOCALAPPDATA\GpAutoLive\locks\csharp-instance.lock" 0 app_stopped
  System::Call 'kernel32::CreateFileW(w "$LOCALAPPDATA\GpAutoLive\locks\csharp-instance.lock", i 0x80000000, i 0, p 0, i 3, i 0x80, p 0) p .r0'
  ${If} $0 == -1
    MessageBox MB_ICONSTOP|MB_OK "GpAutoLive 正在运行。请退出程序后重试。"
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  app_stopped:
  IfFileExists "${APP_EXE}" 0 app_binary_stopped
  System::Call 'kernel32::CreateFileW(w "${APP_EXE}", i 0x00010000, i 0, p 0, i 3, i 0x80, p 0) p .r0'
  ${If} $0 == -1
    MessageBox MB_ICONSTOP|MB_OK "已安装的 GpAutoLive 正在其他会话中运行，或安装目录不可写。请退出程序后重试。"
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  app_binary_stopped:
FunctionEnd

Function .onInit
  SetRegView 64
  SetShellVarContext all
  Call EnsureSupportedSystem
  Call EnsureAppStopped
FunctionEnd

Section "GpAutoLive" SecMain
  SetShellVarContext all
  SetRegView 64

  SetOutPath "$PLUGINSDIR"
  File "/oname=${DOTNET_RUNTIME_FILE}" "${DOTNET_RUNTIME_PATH}"
  DetailPrint "正在安装或修复 .NET 10 Windows Desktop Runtime..."
  ExecWait '"$PLUGINSDIR\${DOTNET_RUNTIME_FILE}" /install /quiet /norestart' $0
  ${If} $0 == 3010
    SetRebootFlag true
  ${ElseIf} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK ".NET 10 Windows Desktop Runtime 安装失败，错误码：$0。"
    Abort
  ${EndIf}

  IfFileExists "$INSTDIR\app\GpAutoLive.exe" recovery_complete 0
  IfFileExists "$INSTDIR\.rollback\GpAutoLive.exe" 0 recovery_complete
  Rename "$INSTDIR\.rollback" "$INSTDIR\app"
  IfErrors recovery_failed
  Goto recovery_complete
  recovery_failed:
    MessageBox MB_ICONSTOP|MB_OK "无法恢复上一次安装保留的版本，请检查安装目录权限。"
    Abort
  recovery_complete:

  RMDir /r "$INSTDIR\.staging-${PACKAGE_VERSION}"
  CreateDirectory "$INSTDIR\.staging-${PACKAGE_VERSION}"
  SetOutPath "$INSTDIR\.staging-${PACKAGE_VERSION}"
  File /r "${APP_PACKAGE_ROOT}\*.*"
  SetOutPath "$INSTDIR"
  ClearErrors
  FileOpen $0 "$INSTDIR\.staging-${PACKAGE_VERSION}\GpAutoLive.control-plane-profile" w
  IfErrors profile_write_failed
  FileWrite $0 "${CONTROL_PLANE_PROFILE}$\r$\n"
  IfErrors profile_close_and_fail
  FileClose $0
  Goto profile_write_complete
  profile_close_and_fail:
  FileClose $0
  profile_write_failed:
  RMDir /r "$INSTDIR\.staging-${PACKAGE_VERSION}"
  MessageBox MB_ICONSTOP|MB_OK "无法写入控制面包配置；旧版本已保留。"
  Abort
  profile_write_complete:

  RMDir /r "$INSTDIR\.rollback"
  IfFileExists "$INSTDIR\app\GpAutoLive.exe" 0 activate_new
  Rename "$INSTDIR\app" "$INSTDIR\.rollback"
  IfErrors activation_failed

  activate_new:
  Rename "$INSTDIR\.staging-${PACKAGE_VERSION}" "$INSTDIR\app"
  IfErrors restore_previous
  Goto activation_complete

  restore_previous:
  IfFileExists "$INSTDIR\.rollback\GpAutoLive.exe" 0 activation_failed
  Rename "$INSTDIR\.rollback" "$INSTDIR\app"
  activation_failed:
  MessageBox MB_ICONSTOP|MB_OK "无法激活新版本，旧版本已尽量保留。请检查磁盘空间和目录权限。"
  Abort

  activation_complete:
  ClearErrors
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\GpAutoLive"
  CreateShortcut "$SMPROGRAMS\GpAutoLive\GpAutoLive.lnk" "${APP_EXE}"
  CreateShortcut "$SMPROGRAMS\GpAutoLive\卸载 GpAutoLive.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\GpAutoLive.lnk" "${APP_EXE}"

  WriteRegStr HKLM "${PRODUCT_REG_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "DisplayVersion" "${PACKAGE_VERSION}"
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "DisplayIcon" "${APP_EXE}"
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_REG_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_REG_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_REG_KEY}" "NoRepair" 1
  IfErrors registration_failed
  RMDir /r "$INSTDIR\.rollback"
  Goto installation_complete

  registration_failed:
  Delete "$DESKTOP\GpAutoLive.lnk"
  Delete "$SMPROGRAMS\GpAutoLive\GpAutoLive.lnk"
  Delete "$SMPROGRAMS\GpAutoLive\卸载 GpAutoLive.lnk"
  RMDir "$SMPROGRAMS\GpAutoLive"
  DeleteRegKey HKLM "${UNINSTALL_REG_KEY}"
  DeleteRegKey HKLM "${PRODUCT_REG_KEY}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\app"
  IfFileExists "$INSTDIR\.rollback\GpAutoLive.exe" 0 registration_rollback_complete
  Rename "$INSTDIR\.rollback" "$INSTDIR\app"
  registration_rollback_complete:
  MessageBox MB_ICONSTOP|MB_OK "快捷方式或卸载登记写入失败，安装已回滚。"
  Abort

  installation_complete:
SectionEnd

Function un.EnsureAppStopped
  IfFileExists "$LOCALAPPDATA\GpAutoLive\locks\csharp-instance.lock" 0 un_app_stopped
  System::Call 'kernel32::CreateFileW(w "$LOCALAPPDATA\GpAutoLive\locks\csharp-instance.lock", i 0x80000000, i 0, p 0, i 3, i 0x80, p 0) p .r0'
  ${If} $0 == -1
    MessageBox MB_ICONSTOP|MB_OK "GpAutoLive 正在运行。请退出程序后重试。"
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  un_app_stopped:
  IfFileExists "${APP_EXE}" 0 un_app_binary_stopped
  System::Call 'kernel32::CreateFileW(w "${APP_EXE}", i 0x00010000, i 0, p 0, i 3, i 0x80, p 0) p .r0'
  ${If} $0 == -1
    MessageBox MB_ICONSTOP|MB_OK "已安装的 GpAutoLive 正在其他会话中运行，或安装目录不可写。请退出程序后重试。"
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  un_app_binary_stopped:
FunctionEnd

Section "Uninstall"
  SetShellVarContext all
  SetRegView 64
  Call un.EnsureAppStopped
  RMDir /r "$INSTDIR\.uninstall"
  IfFileExists "$INSTDIR\app\GpAutoLive.exe" 0 uninstall_payload_removed
  Rename "$INSTDIR\app" "$INSTDIR\.uninstall"
  IfErrors uninstall_payload_failed
  RMDir /r "$INSTDIR\.uninstall"
  IfFileExists "$INSTDIR\.uninstall\*.*" uninstall_payload_failed 0
  Goto uninstall_payload_removed
  uninstall_payload_failed:
    MessageBox MB_ICONSTOP|MB_OK "应用文件未能完整移除；卸载登记和用户数据均保持不变，请重试。"
    Abort
  uninstall_payload_removed:
  RMDir /r "$INSTDIR\.rollback"
  RMDir /r "$INSTDIR\.staging-${PACKAGE_VERSION}"
  Delete "$DESKTOP\GpAutoLive.lnk"
  Delete "$SMPROGRAMS\GpAutoLive\GpAutoLive.lnk"
  Delete "$SMPROGRAMS\GpAutoLive\卸载 GpAutoLive.lnk"
  RMDir "$SMPROGRAMS\GpAutoLive"
  DeleteRegKey HKLM "${UNINSTALL_REG_KEY}"
  DeleteRegKey HKLM "${PRODUCT_REG_KEY}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
