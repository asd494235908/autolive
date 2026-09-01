!include "${__FILEDIR__}\..\..\third_party\akvirtualcamera\installer\akvirtualcamera-components.nsh"

!macro NSIS_HOOK_POSTINSTALL
  IfFileExists "$INSTDIR\portaudio\portaudio_x64.dll" portaudio_dll_present portaudio_dll_missing
  portaudio_dll_present:
    CopyFiles /SILENT "$INSTDIR\portaudio\portaudio_x64.dll" "$INSTDIR"
    Goto portaudio_dll_ready
  portaudio_dll_missing:
    MessageBox MB_ICONSTOP|MB_OK "PortAudio runtime is missing. Please reinstall the application."
    Abort
  portaudio_dll_ready:
  Call GPAkVCamInstall
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Call un.GPAkVCamUninstall
  Delete "$INSTDIR\portaudio_x64.dll"
!macroend
