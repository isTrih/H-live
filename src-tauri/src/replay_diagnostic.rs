//! Opt-in, isolated real WebView2/IPC/decoder diagnostic. No normal-app hooks.
use crate::{backend::Backend,replay_transport};
use serde_json::{Value,json};
use std::{borrow::Cow,fs,path::PathBuf,sync::Arc};
use tauri::{Manager,State,utils::assets::{AssetKey,CspHash}};

struct Probe {report:PathBuf,mode:String,mp4:PathBuf}
struct Assets {script:Vec<u8>,fallback:Option<Box<dyn tauri::Assets<tauri::Wry>>>}
impl tauri::Assets<tauri::Wry> for Assets {
    fn get(&self,key:&AssetKey)->Option<Cow<'_,[u8]>> {
        match key.as_ref(){"/index.html"|"/"=>Some(Cow::Borrowed(b"<!doctype html><html><head><meta charset=\"utf-8\"></head><body><script src=\"/diagnostic.js\"></script></body></html>")),"/diagnostic.js"=>Some(Cow::Borrowed(&self.script)),_=>self.fallback.as_deref()?.get(key)}
    }
    fn iter(&self)->Box<tauri::utils::assets::AssetsIter<'_>> {Box::new(std::iter::empty())}
    fn csp_hashes(&self,_:&AssetKey)->Box<dyn Iterator<Item=CspHash<'_>>+'_>{Box::new(std::iter::empty())}
}
#[tauri::command]
fn replay_diagnostic_config(state:State<'_,Probe>)->Value {json!({"mode":state.mode})}
#[tauri::command]
fn replay_diagnostic_mp4(state:State<'_,Probe>)->Result<tauri::ipc::Response,String>{
    fs::read(&state.mp4).map(tauri::ipc::Response::new).map_err(|_|"诊断样本不可读".into())
}
#[tauri::command]
fn replay_diagnostic_capture(state:State<'_,Probe>)->Result<u64,String>{
    let path=state.report.parent().ok_or("诊断目录不可用")?.join("replay-native-gpu-first-frame.png");
    crate::native_replay::screenshot(&path)?;
    fs::metadata(path).map(|m|m.len()).map_err(|_|"原生首帧尚不可用".into())
}
#[tauri::command]
async fn replay_diagnostic_window(app:tauri::AppHandle,state:State<'_,Probe>)->Result<u64,String>{
    let path=state.report.parent().ok_or("诊断目录不可用")?.join("replay-native-window.bmp");
    tauri::async_runtime::spawn_blocking(move||{
        use windows_sys::Win32::{Foundation::RECT,Graphics::Gdi::*,UI::WindowsAndMessaging::GetClientRect};
        #[link(name="user32")]
        unsafe extern "system"{fn PrintWindow(hwnd:windows_sys::Win32::Foundation::HWND,dc:HDC,flags:u32)->i32;}
        let hwnd=app.get_window("main").ok_or("诊断窗口不存在")?.hwnd().map_err(|_|"诊断窗口句柄不可用")?.0;
        unsafe{
            let mut rect:RECT=std::mem::zeroed();GetClientRect(hwnd,&mut rect);let width=rect.right;let height=rect.bottom;
            if width<=0||height<=0||width>10000||height>10000{return Err("诊断截图尺寸无效".into());}
            let screen=GetDC(hwnd);let dc=CreateCompatibleDC(screen);let bitmap=CreateCompatibleBitmap(screen,width,height);let old=SelectObject(dc,bitmap as _);
            let ok=PrintWindow(hwnd,dc,2);SelectObject(dc,old);
            let mut info:BITMAPINFO=std::mem::zeroed();info.bmiHeader.biSize=std::mem::size_of::<BITMAPINFOHEADER>() as u32;info.bmiHeader.biWidth=width;info.bmiHeader.biHeight=-height;info.bmiHeader.biPlanes=1;info.bmiHeader.biBitCount=32;info.bmiHeader.biCompression=BI_RGB;
            let mut pixels=vec![0u8;(width*height*4)as usize];let rows=GetDIBits(dc,bitmap,0,height as u32,pixels.as_mut_ptr().cast(),&mut info,DIB_RGB_COLORS);
            DeleteObject(bitmap as _);DeleteDC(dc);ReleaseDC(hwnd,screen);
            if ok==0||rows==0{return Err("诊断窗口截图失败".into());}
            let mut file=Vec::with_capacity(54+pixels.len());file.extend_from_slice(b"BM");file.extend_from_slice(&((54+pixels.len())as u32).to_le_bytes());file.extend_from_slice(&[0u8;4]);file.extend_from_slice(&54u32.to_le_bytes());file.extend_from_slice(&40u32.to_le_bytes());file.extend_from_slice(&width.to_le_bytes());file.extend_from_slice(&(-height).to_le_bytes());file.extend_from_slice(&1u16.to_le_bytes());file.extend_from_slice(&32u16.to_le_bytes());file.extend_from_slice(&[0u8;24]);file.extend_from_slice(&pixels);let len=file.len()as u64;fs::write(path,file).map_err(|_|"诊断截图不可写")?;Ok(len)
        }
    }).await.map_err(|_|"诊断截图任务失败".to_owned())?
}
#[tauri::command]
fn replay_diagnostic_overlay(app:tauri::AppHandle)->Result<(),String>{
    // This command exists only in the explicit diagnostic app. It exercises the
    // real child document's controls and reports fixed DOM booleans, no media.
    app.get_webview("replay-overlay").ok_or("诊断弹幕图层不存在")?.eval(r#"
      (async()=>{
        const control=document.querySelector('[data-action="timeline-mute"]');
        const emit=window.__TAURI_INTERNALS__.invoke;
        const payload={controls:!!control,danmaku:!!document.querySelector('.timeline-danmaku-item'),seek:!!document.querySelector('#timeline-media-seek'),control_groups:document.querySelectorAll('.timeline-controls').length,time_labels:document.querySelectorAll('.timeline-position').length,danmaku_items:document.querySelectorAll('.timeline-danmaku-item').length,visible:getComputedStyle(document.querySelector('#native-overlay')).visibility==='visible'};
        await emit('plugin:event|emit_to',{target:{kind:'AnyLabel',label:'main'},event:'replay-diagnostic-overlay',payload});
        control?.click();
      })();
    "#).map_err(|_|"诊断弹幕图层执行失败".into())
}
#[tauri::command]
fn replay_diagnostic_finish(app:tauri::AppHandle,state:State<'_,Probe>,report:Value)->Result<(),String>{
    let bytes=serde_json::to_vec_pretty(&report).map_err(|_|"诊断报告编码失败")?;
    if bytes.len()>65536{return Err("诊断报告超出限制".into());}
    fs::write(&state.report,bytes).map_err(|_|"诊断报告不可写")?;
    app.exit(if report["first_frame"]==true{0}else{3});Ok(())
}
pub(crate) fn run(arguments:&[String]) {
    let report=PathBuf::from(arguments.get(2).expect("diagnostic report path"));
    let directory=report.parent().expect("diagnostic report parent").to_owned();
    fs::create_dir_all(&directory).expect("diagnostic directory");
    let mode=arguments.get(3).cloned().unwrap_or_else(||"remote".into());
    let script=fs::read(directory.join("replay-diagnostic/diagnostic.js")).expect("build diagnostic JS first");
    let backend=Arc::new(Backend::new());
    if mode!="h264-probe"{
        let runtime=backend.get().expect("isolated backend");
        let key=std::env::var("INTERNAL_API_TOKEN").expect("diagnostic key must be provided through the environment");
        runtime.request("community_configure",json!({"enabled":true,"api_key":key})).expect("diagnostic community configuration");
    }
    let mut context=tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().build.dev_url=None;
    let fallback=std::mem::replace(&mut context.assets,Box::new(Assets{script:Vec::new(),fallback:None}));
    context.set_assets(Box::new(Assets{script,fallback:Some(fallback)}));
    let webview_data=directory.join(format!("replay-diagnostic-webview-{mode}"));
    let visible=mode=="native";
    tauri::Builder::default().manage(backend).manage(Probe{report,mode,mp4:directory.join("replay-native-h264-probe.mp4")})
        .invoke_handler(tauri::generate_handler![crate::backend::backend_request,replay_transport::replay_media_fetch,replay_diagnostic_config,replay_diagnostic_mp4,replay_diagnostic_capture,replay_diagnostic_window,replay_diagnostic_overlay,replay_diagnostic_finish,crate::native_replay::native_replay_open,crate::native_replay::native_replay_status,crate::native_replay::native_replay_action,crate::native_replay::native_replay_bounds,crate::native_replay::native_replay_close])
        .setup(move|app|{
            tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::App("index.html".into()))
                .title("氢看直播 · 独立播放诊断").inner_size(720.,480.).transparent(true).decorations(false).shadow(false).visible(visible).data_directory(webview_data).build()?;
            Ok(())
        }).run(context).expect("diagnostic WebView2 startup");
}
