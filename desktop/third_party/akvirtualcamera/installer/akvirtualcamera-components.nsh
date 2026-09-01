; GpAutoLive AkVirtualCamera component lifecycle for the Tauri NSIS bundle.
;
; The release gate produces release-ready.json only after artifact hashes,
; Authenticode evidence, GPL materials and compatibility evidence pass. A
; development/test bundle therefore takes the no-op path and cannot register
; an unsigned or partial camera. The hook never force-terminates unrelated
; removes an installation whose registry owner is not this product.

!include LogicLib.nsh
!include x64.nsh

!define GPAKVCAM_ROOT "$INSTDIR\akvirtualcamera"
!define GPAKVCAM_REGKEY "SOFTWARE\Webcamoid\VirtualCamera"
!define GPAKVCAM_DEVICE_ID "GpAutoLiveCamera"
!define GPAKVCAM_DEVICE_DESCRIPTION "GpAutoLive Camera"

Var GpAkVcamRoot
Var GpAkVcamPreviousPath64
Var GpAkVcamPreviousPath32
Var GpAkVcamDeviceAdded
Var GpAkVcamRegisteredX64
Var GpAkVcamRegisteredX86

Function GPAkVCamReadRegistryOwners
  StrCpy $GpAkVcamRoot "${GPAKVCAM_ROOT}"
  SetRegView 64
  ReadRegStr $GpAkVcamPreviousPath64 HKLM "${GPAKVCAM_REGKEY}" "installPath"
  SetRegView 32
  ReadRegStr $GpAkVcamPreviousPath32 HKLM "${GPAKVCAM_REGKEY}" "installPath"
  SetRegView default
FunctionEnd

Function GPAkVCamRestoreRegistry
  SetRegView 64
  ${If} $GpAkVcamPreviousPath64 == ""
    DeleteRegValue HKLM "${GPAKVCAM_REGKEY}" "installPath"
  ${Else}
    WriteRegStr HKLM "${GPAKVCAM_REGKEY}" "installPath" "$GpAkVcamPreviousPath64"
  ${EndIf}
  SetRegView 32
  ${If} $GpAkVcamPreviousPath32 == ""
    DeleteRegValue HKLM "${GPAKVCAM_REGKEY}" "installPath"
  ${Else}
    WriteRegStr HKLM "${GPAKVCAM_REGKEY}" "installPath" "$GpAkVcamPreviousPath32"
  ${EndIf}
  SetRegView default
FunctionEnd

Function un.GPAkVCamReadRegistryOwners
  StrCpy $GpAkVcamRoot "${GPAKVCAM_ROOT}"
  SetRegView 64
  ReadRegStr $GpAkVcamPreviousPath64 HKLM "${GPAKVCAM_REGKEY}" "installPath"
  SetRegView 32
  ReadRegStr $GpAkVcamPreviousPath32 HKLM "${GPAKVCAM_REGKEY}" "installPath"
  SetRegView default
FunctionEnd

Function un.GPAkVCamRestoreRegistry
  SetRegView 64
  ${If} $GpAkVcamPreviousPath64 == ""
    DeleteRegValue HKLM "${GPAKVCAM_REGKEY}" "installPath"
  ${Else}
    WriteRegStr HKLM "${GPAKVCAM_REGKEY}" "installPath" "$GpAkVcamPreviousPath64"
  ${EndIf}
  SetRegView 32
  ${If} $GpAkVcamPreviousPath32 == ""
    DeleteRegValue HKLM "${GPAKVCAM_REGKEY}" "installPath"
  ${Else}
    WriteRegStr HKLM "${GPAKVCAM_REGKEY}" "installPath" "$GpAkVcamPreviousPath32"
  ${EndIf}
  SetRegView default
FunctionEnd

Function GPAkVCamUnregisterFilters
  ${If} $GpAkVcamRegisteredX64 == 1
    ExecWait '"$SYSDIR\regsvr32.exe" /s /u "${GPAKVCAM_ROOT}\x64\AkVirtualCamera.dll"' $R0
  ${EndIf}
  ${If} $GpAkVcamRegisteredX86 == 1
    ExecWait '"$WINDIR\SysWOW64\regsvr32.exe" /s /u "${GPAKVCAM_ROOT}\x86\AkVirtualCamera.dll"' $R0
  ${EndIf}
  StrCpy $GpAkVcamRegisteredX64 0
  StrCpy $GpAkVcamRegisteredX86 0
FunctionEnd

Function GPAkVCamRollbackInstall
  ${If} $GpAkVcamDeviceAdded == 1
    IfFileExists "${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" 0 +3
      ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" remove-device "${GPAKVCAM_DEVICE_ID}"' $R0
      ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" update' $R0
  ${EndIf}
  Call GPAkVCamUnregisterFilters
  Call GPAkVCamRestoreRegistry
  StrCpy $GpAkVcamDeviceAdded 0
FunctionEnd

Function GPAkVCamInstall
  ; Missing marker is the expected development/test path.
  IfFileExists "${GPAKVCAM_ROOT}\release-ready.json" 0 GPAkVCamInstallDone
  Call GPAkVCamReadRegistryOwners

  ; Never overwrite another AkVCam installation in either registry view.
  ${If} $GpAkVcamPreviousPath64 != ""
    ${If} $GpAkVcamPreviousPath64 != $GpAkVcamRoot
      Goto GPAkVCamInstallFail
    ${EndIf}
  ${EndIf}
  ${If} $GpAkVcamPreviousPath32 != ""
    ${If} $GpAkVcamPreviousPath32 != $GpAkVcamRoot
      Goto GPAkVCamInstallFail
    ${EndIf}
  ${EndIf}

  ; The marker is only consumed after the release verifier has checked the
  ; files. Keep exact component names and architecture-specific registration.
  IfFileExists "${GPAKVCAM_ROOT}\x64\AkVirtualCamera.dll" 0 GPAkVCamInstallFail
  IfFileExists "${GPAKVCAM_ROOT}\x64\AkVCamAssistant.exe" 0 GPAkVCamInstallFail
  IfFileExists "${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" 0 GPAkVCamInstallFail
  IfFileExists "${GPAKVCAM_ROOT}\x86\AkVirtualCamera.dll" 0 GPAkVCamInstallFail
  IfFileExists "${GPAKVCAM_ROOT}\bin\akvirtualcamera-sidecar-x64.exe" 0 GPAkVCamInstallFail
  IfFileExists "${GPAKVCAM_ROOT}\bin\vcam_capi.dll" 0 GPAkVCamInstallFail

  SetRegView 64
  WriteRegStr HKLM "${GPAKVCAM_REGKEY}" "installPath" "$GpAkVcamRoot"
  SetRegView 32
  WriteRegStr HKLM "${GPAKVCAM_REGKEY}" "installPath" "$GpAkVcamRoot"
  SetRegView default

  ; Mark the filter as potentially registered before invoking regsvr32 so a
  ; partial registration is still cleaned up if regsvr32 returns an error.
  StrCpy $GpAkVcamRegisteredX64 1
  ExecWait '"$SYSDIR\regsvr32.exe" /s "${GPAKVCAM_ROOT}\x64\AkVirtualCamera.dll"' $R0
  ${If} $R0 != 0
    Goto GPAkVCamInstallFail
  ${EndIf}

  ${If} ${RunningX64}
    StrCpy $GpAkVcamRegisteredX86 1
    ExecWait '"$WINDIR\SysWOW64\regsvr32.exe" /s "${GPAKVCAM_ROOT}\x86\AkVirtualCamera.dll"' $R0
    ${If} $R0 != 0
      Goto GPAkVCamInstallFail
    ${EndIf}
  ${EndIf}

  ; AkVCamManager launches the matching Assistant on demand. Existing devices
  ; are updated in place, while a newly created device is tracked for rollback.
  ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" add-device -i "${GPAKVCAM_DEVICE_ID}" "${GPAKVCAM_DEVICE_DESCRIPTION}"' $R0
  ${If} $R0 == 0
    StrCpy $GpAkVcamDeviceAdded 1
  ${Else}
    ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" set-description "${GPAKVCAM_DEVICE_ID}" "${GPAKVCAM_DEVICE_DESCRIPTION}"' $R0
    ${If} $R0 != 0
      Goto GPAkVCamInstallFail
    ${EndIf}
  ${EndIf}
  ; Data mode and direct mode are privileged AkVCam preferences. Configure
  ; them once while the NSIS installer is elevated so the runtime sidecar can
  ; remain unelevated and recover without prompting for UAC.
  ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" set-data-mode mmap' $R0
  ${If} $R0 != 0
    Goto GPAkVCamInstallFail
  ${EndIf}
  ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" set-direct-mode "${GPAKVCAM_DEVICE_ID}" 1' $R0
  ${If} $R0 != 0
    Goto GPAkVCamInstallFail
  ${EndIf}
  ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" update' $R0
  ${If} $R0 != 0
    Goto GPAkVCamInstallFail
  ${EndIf}
  Goto GPAkVCamInstallDone

GPAkVCamInstallFail:
  Call GPAkVCamRollbackInstall
  MessageBox MB_ICONSTOP|MB_OK "AkVirtualCamera 组件安装失败，已回滚注册和设备状态。"
  Abort

GPAkVCamInstallDone:
FunctionEnd

Function un.GPAkVCamUninstall
  IfFileExists "${GPAKVCAM_ROOT}\release-ready.json" 0 GPAkVCamUninstallDone
  Call un.GPAkVCamReadRegistryOwners

  ; If another product replaced the registry owner, leave it untouched.
  ${If} $GpAkVcamPreviousPath64 != $GpAkVcamRoot
    Goto GPAkVCamUninstallDone
  ${EndIf}
  ${If} $GpAkVcamPreviousPath32 != $GpAkVcamRoot
    Goto GPAkVCamUninstallDone
  ${EndIf}

  ; The producer belongs to GpAutoLive and downstream clients may still hold
  ; the DirectShow device. Do not force-kill unrelated processes: require the
  ; user to close GpAutoLive and all camera consumers before unregistering.
  MessageBox MB_ICONEXCLAMATION|MB_YESNO "卸载 GpAutoLive Camera 前，请先退出 GpAutoLive 以及 Chrome、Teams、Zoom、OBS 等所有摄像头应用。继续卸载可能会中断正在使用摄像头的应用。是否继续？" IDYES GPAkVCamUninstallConfirmed
  Abort
GPAkVCamUninstallConfirmed:

  IfFileExists "${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" 0 GPAkVCamUninstallFilters
    ; Never remove all devices: other AkVCam devices may belong to another app.
    ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" remove-device "${GPAKVCAM_DEVICE_ID}"' $R0
    ExecWait '"${GPAKVCAM_ROOT}\x64\AkVCamManager.exe" update' $R0

GPAkVCamUninstallFilters:
  IfFileExists "${GPAKVCAM_ROOT}\x64\AkVirtualCamera.dll" 0 +2
    ExecWait '"$SYSDIR\regsvr32.exe" /s /u "${GPAKVCAM_ROOT}\x64\AkVirtualCamera.dll"' $R0
  IfFileExists "${GPAKVCAM_ROOT}\x86\AkVirtualCamera.dll" 0 +2
    ExecWait '"$WINDIR\SysWOW64\regsvr32.exe" /s /u "${GPAKVCAM_ROOT}\x86\AkVirtualCamera.dll"' $R0
  Call un.GPAkVCamRestoreRegistry

GPAkVCamUninstallDone:
FunctionEnd
