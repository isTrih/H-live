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
