param(
  [ValidateSet('baseline','hide-overlay','video-top','layer-order','no-sibling-clip','layer-no-clip','hide-main')][string]$Mode='baseline',
  [string]$Executable='',
  [ValidateSet('playing','paused','fullscreen')][string]$Stage='playing',
  [switch]$Complete
)
# Opt-in Windows replay diagnostic. Uses its own app/profile and captures only
# that test window. A known colorful fixture is required for the pixel oracle.
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent $PSScriptRoot
$runRoot=Join-Path $projectRoot ('build/native-visible-' + $Mode + '-' + $Stage + '-' + [DateTime]::Now.ToString('HHmmssfff'))
New-Item -ItemType Directory -Path (Join-Path $runRoot 'replay-diagnostic') -Force | Out-Null
$js=[IO.File]::ReadAllText((Join-Path $projectRoot 'build/replay-diagnostic/diagnostic.js'))
$needle='report.first_frame = report.screenshot_bytes > 0;'
if (-not $js.Contains($needle)) { throw 'Diagnostic first-frame marker changed' }
if($Stage -eq 'playing'){
  $js=$js.Replace($needle,$needle+'await new Promise(resolve=>setTimeout(resolve,15000));')
}else{
  $capture='report.screenshot_bytes = await invoke("replay_diagnostic_capture");'
  if(-not $js.Contains($capture)){throw 'Diagnostic capture marker changed'}
  $js=$js.Replace($capture,'report.screenshot_bytes = 1;')
  $marker=if($Stage -eq 'paused'){'report.pause_ok = (await invoke("native_replay_status")).paused === true;'}else{'report.fullscreen_ok = (await invoke("native_replay_status")).fullscreen === true;'}
  if(-not $js.Contains($marker)){throw 'Diagnostic stage marker changed'}
  $js=$js.Replace($marker,$marker+'await new Promise(resolve=>setTimeout(resolve,500));'+$capture+'await new Promise(resolve=>setTimeout(resolve,15000));')
}
[IO.File]::WriteAllText((Join-Path $runRoot 'replay-diagnostic/diagnostic.js'),$js)
Add-Type -AssemblyName System.Drawing
$drawingReferences=@(Get-ChildItem -LiteralPath (Join-Path $PSHOME 'ref') -Filter '*.dll' | Select-Object -ExpandProperty FullName)+@([System.Drawing.Bitmap].Assembly.Location,(Join-Path $PSHOME 'System.Private.Windows.GdiPlus.dll'),(Join-Path $PSHOME 'System.Private.Windows.Core.dll'))
Add-Type -ReferencedAssemblies $drawingReferences -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
public static class VideoVisibility {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left,top,right,bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int x,y; }
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int show);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h,uint command);
  [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetStyle(IntPtr h,int index);
  [DllImport("user32.dll",EntryPoint="SetWindowLongPtrW")] public static extern IntPtr SetStyle(IntPtr h,int index,IntPtr value);
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr h,System.Text.StringBuilder text,int count);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
  public static object[] Children(IntPtr parent) {
    var rows=new List<object>();var h=GetWindow(parent,5);
    while(h!=IntPtr.Zero){var b=new System.Text.StringBuilder(256);GetClassName(h,b,256);Rect r;GetClientRect(h,out r);rows.Add(new {hwnd=h.ToInt64(),kind=b.ToString(),width=r.right,height=r.bottom,visible=IsWindowVisible(h)});h=GetWindow(h,2);}
    return rows.ToArray();
  }
  public static double Capture(IntPtr hwnd,string destination) {
    SetThreadDpiAwarenessContext(new IntPtr(-4));
    Rect r;GetClientRect(hwnd,out r);var p=new Point();ClientToScreen(hwnd,ref p);
    using(var image=new Bitmap(r.right,r.bottom)){
      using(var g=Graphics.FromImage(image)){g.CopyFromScreen(p.x,p.y,0,0,image.Size,CopyPixelOperation.SourceCopy);}
      image.Save(destination,ImageFormat.Png);double minimum=1;
      for(int row=0;row<2;row++)for(int column=0;column<2;column++){
        int colorful=0,total=0;
        for(int y=120+row*90;y<Math.Min(r.bottom-100,210+row*90);y+=3)for(int x=40+column*280;x<Math.Min(r.right-50,320+column*280);x+=3){var c=image.GetPixel(x,y);int high=Math.Max(c.R,Math.Max(c.G,c.B)),low=Math.Min(c.R,Math.Min(c.G,c.B));if(high-low>24&&high>45)colorful++;total++;}
        minimum=Math.Min(minimum,total>0?(double)colorful/total:0);
      }
      return minimum;
    }
  }
}
'@
$priorDataHome=$env:DOUYIN_MONITOR_HOME
$priorReplayToken=$env:INTERNAL_API_TOKEN
$report=Join-Path $runRoot 'report.json'
if(-not $Executable){$Executable=Join-Path $projectRoot 'dist/DouyinLiveMonitor/DouyinLiveMonitor.exe'}
$process=$null
try {
  $env:DOUYIN_MONITOR_HOME=Join-Path $runRoot 'runtime'
  foreach($line in (Get-Content -LiteralPath (Join-Path $projectRoot '.env'))){if($line -match '^\s*INTERNAL_API_TOKEN\s*=\s*(.+)\s*$'){$env:INTERNAL_API_TOKEN=$Matches[1].Trim().Trim('"').Trim("'")}}
  $process=Start-Process -FilePath $Executable -ArgumentList @('--replay-diagnostic',('"'+$report+'"'),'native') -RedirectStandardError (Join-Path $runRoot 'stderr.log') -WindowStyle Hidden -PassThru
  $limit=[DateTime]::UtcNow.AddSeconds(55)
  while(-not (Test-Path -LiteralPath (Join-Path $runRoot 'replay-native-gpu-first-frame.png'))){if($process.HasExited){throw ('Diagnostic exited ' + $process.ExitCode)};if([DateTime]::UtcNow -gt $limit){throw 'No decoded frame within deadline'};Start-Sleep -Milliseconds 100}
  $process.Refresh();$hwnd=$process.MainWindowHandle
  if($hwnd -eq [IntPtr]::Zero){throw 'Diagnostic window handle not available'}
  [VideoVisibility]::SetWindowPos($hwnd,[IntPtr](-1),0,0,0,0,0x53) | Out-Null
  $children=@([VideoVisibility]::Children($hwnd))
  if($Mode -eq 'hide-overlay'){foreach($child in $children){if($child.kind -eq 'WRY_WEBVIEW'){[VideoVisibility]::ShowWindow([IntPtr]$child.hwnd,0)|Out-Null;break}}}
  if($Mode -eq 'video-top'){foreach($child in $children){if($child.kind -eq 'Static' -or $child.kind -eq 'STATIC'){[VideoVisibility]::SetWindowPos([IntPtr]$child.hwnd,[IntPtr]::Zero,0,0,0,0,0x53)|Out-Null;break}}}
  if($Mode -in @('layer-order','layer-no-clip')){
    foreach($child in $children){if($child.kind -eq 'Static' -or $child.kind -eq 'STATIC'){[VideoVisibility]::SetWindowPos([IntPtr]$child.hwnd,[IntPtr]::Zero,0,0,0,0,0x53)|Out-Null;break}}
    foreach($child in $children){if($child.kind -eq 'WRY_WEBVIEW'){[VideoVisibility]::SetWindowPos([IntPtr]$child.hwnd,[IntPtr]::Zero,0,0,0,0,0x53)|Out-Null;break}}
  }
  if($Mode -in @('no-sibling-clip','layer-no-clip')){foreach($child in $children){if($child.kind -eq 'Static' -or $child.kind -eq 'STATIC'){$style=[VideoVisibility]::GetStyle([IntPtr]$child.hwnd,-16).ToInt64();[VideoVisibility]::SetStyle([IntPtr]$child.hwnd,-16,[IntPtr]($style -band (-bnot 0x04000000)))|Out-Null;[VideoVisibility]::SetWindowPos([IntPtr]$child.hwnd,[IntPtr]::Zero,0,0,0,0,0x37)|Out-Null;break}}}
  if($Mode -eq 'hide-main'){foreach($child in $children){if($child.kind -eq 'WRY_WEBVIEW' -and $child.width -eq 720){[VideoVisibility]::ShowWindow([IntPtr]$child.hwnd,0)|Out-Null;break}}}
  Start-Sleep -Seconds 10
  $ratio=[VideoVisibility]::Capture($hwnd,(Join-Path $runRoot 'screen.png'))
  $result=@{mode=$Mode;stage=$Stage;minimum_tile_color_ratio=$ratio;visible_video=($ratio -gt 0.08);children=$children;children_after=@([VideoVisibility]::Children($hwnd));decoded_frame=$true;directory=$runRoot}
  $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runRoot 'visibility.json') -Encoding utf8
  $result | ConvertTo-Json -Depth 6
  if($Complete){
    if(-not $process.WaitForExit(45000)){throw 'Playback interaction regression did not complete'}
    $diagnostic=Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    $diagnostic | ConvertTo-Json -Depth 6
    if(-not $diagnostic.source_hidden -or -not $diagnostic.overlay_dom.visible -or $diagnostic.overlay_dom.control_groups -ne 1 -or $diagnostic.overlay_dom.time_labels -ne 1 -or $diagnostic.overlay_dom.danmaku_items -lt 1 -or $diagnostic.overlay_dom.danmaku_items -gt 16 -or -not $diagnostic.overlay_control_roundtrip -or -not $diagnostic.pause_ok -or -not $diagnostic.seek_ok -or -not $diagnostic.volume_ok -or -not $diagnostic.mute_ok -or -not $diagnostic.fullscreen_ok -or -not $diagnostic.closed_ok){throw 'Playback interaction regression failed'}
  }
  if(-not $result.visible_video){exit 3}
} finally {
  if($process -and -not $process.HasExited){Stop-Process -Id $process.Id}
  $env:INTERNAL_API_TOKEN=$priorReplayToken;$env:DOUYIN_MONITOR_HOME=$priorDataHome
}
