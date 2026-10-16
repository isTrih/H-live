; 0.1.0 starts a single NSIS update channel. Never silently execute an older
; Inno uninstaller or overwrite its installation. User data is outside $INSTDIR.
!macro NSIS_HOOK_PREINSTALL
  Push $0
  Push $1
  Push $2
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{97F3675A-88D2-426D-90DA-CB4B8F831731}_is1" "InstallLocation"
  StrCmp $0 "" qingkan_no_legacy_path
  GetFullPathName $1 "$0\."
  GetFullPathName $2 "$INSTDIR\."
  StrCmp $1 $2 qingkan_reject_legacy qingkan_no_legacy_path
qingkan_no_legacy_path:
  IfFileExists "$INSTDIR\unins000.exe" qingkan_reject_legacy qingkan_safe_install
qingkan_reject_legacy:
  MessageBox MB_OK|MB_ICONSTOP "请选择新的安装目录。此目录属于旧版 Inno 安装，不会自动覆盖或卸载。关闭旧版后，将当前正式版安装到默认的新目录；本地数据会继续保留。" /SD IDOK
  SetErrorLevel 2
  Abort
qingkan_safe_install:
  Pop $2
  Pop $1
  Pop $0
!macroend

; Tauri /UPDATE preserves files not listed by the new installer. Remove only
; the recognized obsolete bundled engine inside this installation directory.
; Persistent config/databases live outside this directory and are untouched.
!macro NSIS_HOOK_POSTINSTALL
  IfFileExists "$INSTDIR\engine\DouyinMonitorEngine.exe" 0 qingkan_engine_done
  Push $0
  Push $1
  GetFullPathName $0 "$INSTDIR\engine"
  GetFullPathName $1 "$INSTDIR"
  StrCmp $0 $1 qingkan_engine_unsafe
  ; Never recursively traverse an engine directory redirected by a junction.
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR\engine") i.r0'
  IntOp $0 $0 & 0x400
  IntCmp $0 0 qingkan_engine_remove qingkan_engine_unsafe qingkan_engine_unsafe
qingkan_engine_remove:
  RMDir /r "$INSTDIR\engine"
  IfFileExists "$INSTDIR\engine\DouyinMonitorEngine.exe" qingkan_engine_unsafe
  Pop $1
  Pop $0
  Goto qingkan_engine_done
qingkan_engine_unsafe:
  Pop $1
  Pop $0
  MessageBox MB_OK|MB_ICONSTOP "旧版采集运行文件未能安全清理，请关闭旧程序后重新安装。聊天归档不受影响。" /SD IDOK
  SetErrorLevel 2
  Abort
qingkan_engine_done:
!macroend
