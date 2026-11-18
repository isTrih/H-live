//! Embedded libmpv/D3D11 video. Pixels remain native; the transparent child
//! WebView contains only controls and danmaku, never frame data over IPC.
#[cfg(windows)]
mod platform {
    use crate::backend::Backend;
    use serde_json::{Value,json};
    use std::{ffi::{c_void,c_char,CStr,CString},path::{Path,PathBuf},sync::{Arc,Mutex,OnceLock,atomic::{AtomicU64,AtomicBool,Ordering}},time::{Duration,Instant}};
    use tauri::{Manager,State,Webview,AppHandle};
    use windows_sys::Win32::{Foundation::{HWND,HMODULE,FreeLibrary},Graphics::Gdi::{CreateRectRgn,SetWindowRgn,DeleteObject},System::LibraryLoader::{LoadLibraryExW,GetProcAddress,GetModuleHandleW,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,LOAD_LIBRARY_SEARCH_DEFAULT_DIRS},UI::WindowsAndMessaging::{CreateWindowExW,DestroyWindow,SetWindowPos,ShowWindow,GetWindow,GW_CHILD,GW_HWNDNEXT,SW_HIDE,SW_SHOWNA,WS_CHILD,WS_VISIBLE,WS_CLIPCHILDREN,SWP_NOACTIVATE,SWP_NOZORDER}};

    type Create=unsafe extern "C" fn()->*mut c_void;
    type Destroy=unsafe extern "C" fn(*mut c_void);
    type Initialize=unsafe extern "C" fn(*mut c_void)->i32;
    type OptionString=unsafe extern "C" fn(*mut c_void,*const c_char,*const c_char)->i32;
    type Property=unsafe extern "C" fn(*mut c_void,*const c_char,i32,*mut c_void)->i32;
    type Command=unsafe extern "C" fn(*mut c_void,*const *const c_char)->i32;
    type Free=unsafe extern "C" fn(*mut c_void);
    type FreeNode=unsafe extern "C" fn(*mut crate::replay_cache::Node);
    #[repr(C)]struct Event{event_id:i32,error:i32,reply_userdata:u64,data:*mut c_void}
    #[repr(C)]struct EndFile{reason:i32,error:i32}
    type WaitEvent=unsafe extern "C" fn(*mut c_void,f64)->*const Event;
    struct Api {library:usize,create:Create,destroy:Destroy,initialize:Initialize,option:OptionString,get:Property,set:Property,command:Command,free:Free,free_node:FreeNode,wait:WaitEvent}
    impl Drop for Api {fn drop(&mut self){unsafe{FreeLibrary(self.library as HMODULE);}}}
    impl Api {
        fn load(path:&Path)->Result<Self,String>{
            use std::os::windows::ffi::OsStrExt;
            let wide:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
            let library=unsafe{LoadLibraryExW(wide.as_ptr(),std::ptr::null_mut(),LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)};
            if library.is_null(){return Err("原生播放器组件不可用，请重新安装最新版".into());}
            macro_rules! symbol {($name:literal,$ty:ty)=>{{
                let pointer=unsafe{GetProcAddress(library,concat!($name,"\0").as_ptr())};
                match pointer {Some(pointer)=>unsafe{std::mem::transmute::<unsafe extern "system" fn()->isize,$ty>(pointer)},None=>{unsafe{FreeLibrary(library);};return Err("原生播放器组件版本不兼容".into());}}
            }}}
            Ok(Self{library:library as usize,create:symbol!("mpv_create",Create),destroy:symbol!("mpv_terminate_destroy",Destroy),initialize:symbol!("mpv_initialize",Initialize),option:symbol!("mpv_set_option_string",OptionString),get:symbol!("mpv_get_property",Property),set:symbol!("mpv_set_property",Property),command:symbol!("mpv_command",Command),free:symbol!("mpv_free",Free),free_node:symbol!("mpv_free_node_contents",FreeNode),wait:symbol!("mpv_wait_event",WaitEvent)})
        }
    }
    struct Player {api:Api,handle:usize,hwnd:usize,overlay:Option<Webview>,overlay_hwnds:Vec<usize>,app:AppHandle,id:u64,fullscreen:bool,started:Instant,error:Mutex<Option<String>>,performance:Mutex<Option<(Instant,Value)>>,file_loaded:AtomicBool,playback_restarted:AtomicBool}
    impl Player {
        fn handle(&self)->*mut c_void{self.handle as *mut c_void}
        fn command(&self,args:&[&str])->Result<(),String>{
            let strings:Result<Vec<_>,_>=args.iter().map(|s|CString::new(*s)).collect();
            let strings=strings.map_err(|_|"原生播放参数无效")?;
            let mut pointers:Vec<_>=strings.iter().map(|s|s.as_ptr()).collect();pointers.push(std::ptr::null());
            if unsafe{(self.api.command)(self.handle(),pointers.as_ptr())}<0{return Err("原生播放操作失败".into());}Ok(())
        }
        fn number(&self,key:&str)->Option<f64>{let key=CString::new(key).ok()?;let mut value: f64=0.;let code=unsafe{(self.api.get)(self.handle(),key.as_ptr(),5,(&mut value as *mut f64).cast())};(code>=0&&value.is_finite()).then_some(value)}
        fn integer(&self,key:&str)->Option<i64>{let key=CString::new(key).ok()?;let mut value=0i64;let code=unsafe{(self.api.get)(self.handle(),key.as_ptr(),4,(&mut value as *mut i64).cast())};(code>=0).then_some(value)}
        fn flag(&self,key:&str)->bool{let key=CString::new(key).unwrap();let mut value=0i32;unsafe{(self.api.get)(self.handle(),key.as_ptr(),3,(&mut value as *mut i32).cast())>=0&&value!=0}}
        fn string(&self,key:&str)->String{
            let key=CString::new(key).unwrap();let mut pointer: *mut c_char=std::ptr::null_mut();
            let code=unsafe{(self.api.get)(self.handle(),key.as_ptr(),1,(&mut pointer as *mut *mut c_char).cast())};
            if code<0||pointer.is_null(){return String::new();}
            let value=unsafe{CStr::from_ptr(pointer)}.to_string_lossy().chars().take(128).collect();unsafe{(self.api.free)(pointer.cast());}value
        }
        fn set_number(&self,key:&str,mut value:f64)->Result<(),String>{let key=CString::new(key).unwrap();if unsafe{(self.api.set)(self.handle(),key.as_ptr(),5,(&mut value as *mut f64).cast())}<0{Err("原生播放设置失败".into())}else{Ok(())}}
        fn set_flag(&self,key:&str,value:bool)->Result<(),String>{let key=CString::new(key).unwrap();let mut value=i32::from(value);if unsafe{(self.api.set)(self.handle(),key.as_ptr(),3,(&mut value as *mut i32).cast())}<0{Err("原生播放设置失败".into())}else{Ok(())}}
        fn cache_snapshot(&self)->Value{
            let mut node=crate::replay_cache::Node::default();
            let mut cache=if unsafe{(self.api.get)(self.handle(),c"demuxer-cache-state".as_ptr(),6,(&mut node as *mut crate::replay_cache::Node).cast())}>=0{
                let value=unsafe{crate::replay_cache::cache_fields(&node)};
                unsafe{(self.api.free_node)(&mut node)};value
            }else{json!({})};
            cache["ahead_seconds"]=json!(self.number("demuxer-cache-duration"));
            cache["buffering"]=json!(self.flag("paused-for-cache"));
            cache["buffering_percent"]=json!(self.number("cache-buffering-state"));
            cache["seeking"]=json!(self.flag("seeking"));
            cache["idle"]=json!(self.flag("demuxer-cache-idle"));
            cache["forward_limit_bytes"]=json!(crate::replay_cache::FORWARD_BYTES);
            cache["back_limit_bytes"]=json!(crate::replay_cache::BACK_BYTES);
            cache["ahead_limit_seconds"]=json!(crate::replay_cache::AHEAD_SECONDS);
            cache
        }
        fn performance_snapshot(&self)->Value{
            let mut cached=self.performance.lock().unwrap();
            if let Some((at,value))=cached.as_ref(){if at.elapsed()<Duration::from_secs(1){return value.clone();}}
            // Numeric measurements distinguish network starvation from decoder
            // or presentation drops; sample at 1 Hz rather than every IPC poll.
            let value=json!({"decoded_fps_estimate":self.number("estimated-vf-fps"),"container_fps":self.number("container-fps"),
                "display_fps":self.number("display-fps"),"dropped_frames":self.integer("frame-drop-count"),
                "decoder_dropped_frames":self.integer("decoder-frame-drop-count"),"delayed_frames":self.integer("vo-delayed-frame-count"),
                "av_sync_seconds":self.number("avsync"),"scaler":self.string("scale"),"downscaler":self.string("dscale")});
            *cached=Some((Instant::now(),value.clone()));value
        }
        fn snapshot(&self)->Value{
            for _ in 0..64{
                let event=unsafe{(self.api.wait)(self.handle(),0.)};
                if event.is_null()||unsafe{(*event).event_id}==0{break;}
                if unsafe{(*event).event_id}==8{self.file_loaded.store(true,Ordering::Release);}
                if unsafe{(*event).event_id}==21{self.playback_restarted.store(true,Ordering::Release);}
                if unsafe{(*event).event_id}==7&&!unsafe{(*event).data}.is_null(){
                    let end=unsafe{&*((*event).data as *const EndFile)};
                    if end.reason==4{let reason=match end.error{-13=>"录播媒体加载失败",-14=>"音频设备初始化失败",-15=>"GPU视频输出初始化失败",-17=>"录播格式不可识别",-18=>"当前设备不支持此录播格式",_=>"原生录播播放失败"};*self.error.lock().unwrap()=Some(format!("{reason}（mpv {}）",end.error));}
                }
            }
            let width=self.integer("video-out-params/w").unwrap_or(0);let height=self.integer("video-out-params/h").unwrap_or(0);
            let frame_ready=self.file_loaded.load(Ordering::Acquire)&&self.playback_restarted.load(Ordering::Acquire)&&width>0&&height>0;
            if !frame_ready&&self.started.elapsed()>Duration::from_secs(45)&&self.error.lock().unwrap().is_none(){let _=self.command(&["stop"]);*self.error.lock().unwrap()=Some("原生录播首帧加载超时，请重试".into());}
            let hwdec=self.string("hwdec-current");let codec=self.string("video-codec");
            json!({"active":true,"id":self.id,"time_pos":self.number("time-pos").unwrap_or(0.),"duration":self.number("duration").unwrap_or(0.),"paused":self.flag("pause"),"volume":self.number("volume").unwrap_or(100.)/100.,"muted":self.flag("mute"),"width":width,"height":height,"frame_ready":frame_ready,"vo_configured":self.flag("vo-configured"),"hwdec_active":!hwdec.is_empty()&&hwdec!="no"&&hwdec!="auto","hwdec":hwdec,"video_codec":codec,"fullscreen":self.fullscreen,"core_idle":self.flag("core-idle"),"eof_reached":self.flag("eof-reached"),"mpv_version":self.string("mpv-version"),"ffmpeg_version":self.string("ffmpeg-version"),"cache":self.cache_snapshot(),"performance":self.performance_snapshot(),"error":self.error.lock().unwrap().clone(),"overlay_native_windows":self.overlay_hwnds.len(),"layer_order_valid":crate::native_layers::valid(self.hwnd,&self.overlay_hwnds)})
        }
    }
    impl Drop for Player {
        fn drop(&mut self){
            if let Some(overlay)=self.overlay.take(){let _=overlay.close();}
            unsafe{(self.api.destroy)(self.handle());}
            let hwnd=self.hwnd;
            if self.fullscreen{if let Some(window)=self.app.get_window("main"){let _=window.set_fullscreen(false);}}
            let _=self.app.run_on_main_thread(move||{unsafe{DestroyWindow(hwnd as HWND);}});
        }
    }
    static PLAYER:OnceLock<Mutex<Option<Player>>>=OnceLock::new();
    static LAST_OBSERVED:OnceLock<Mutex<Option<Value>>>=OnceLock::new();
    static OPEN:Mutex<()>=Mutex::new(());
    static NEXT:AtomicU64=AtomicU64::new(1);
    static RESTACK_PENDING:AtomicBool=AtomicBool::new(false);
    fn slot()->&'static Mutex<Option<Player>>{PLAYER.get_or_init(||Mutex::new(None))}
    fn library(app:&AppHandle)->Result<PathBuf,String>{
        #[cfg(debug_assertions)]
        if let Some(path)=std::env::var_os("QINGKAN_MPV_LIBRARY_PATH"){let path=PathBuf::from(path);if path.is_file(){return Ok(path);}}
        #[cfg(debug_assertions)]
        {let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/tools/media/libmpv-2.dll");if path.is_file(){return Ok(path);}}
        let base=app.path().resource_dir().map_err(|_|"原生播放器目录不可用")?.join("media-tools");
        for name in ["libmpv-2.dll","mpv-2.dll"]{let path=base.join(name);if path.is_file(){return Ok(path);}}
        Err("原生播放器组件缺失，请重新安装最新版".into())
    }
    fn bounds(value:&Value)->Result<(f64,f64,f64,f64),String>{
        let get=|key:&str|value[key].as_f64().filter(|v|v.is_finite()).ok_or("播放器位置无效");
        let (x,y,w,h)=(get("x")?,get("y")?,get("width")?,get("height")?);
        if x.abs()>50000.||y.abs()>50000.||w<0.||h<0.||w>20000.||h>20000.{return Err("播放器位置超出范围".into());}Ok((x,y,w,h))
    }
    fn place(player:&Player,value:&Value)->Result<(),String>{
        let (x,y,w,h)=bounds(value)?;let scale=player.app.get_window("main").ok_or("窗口不可用")?.scale_factor().map_err(|_|"窗口缩放不可用")?;
        let hwnd=player.hwnd;let overlays=player.overlay_hwnds.clone();
        let clip=if let Some(clip)=value.get("clip"){let (cx,cy,cw,ch)=bounds(clip)?;Some((cx.max(0.),cy.max(0.),cw.min(w),ch.min(h)))}else{None};
        player.app.run_on_main_thread(move||{unsafe{SetWindowPos(hwnd as HWND,std::ptr::null_mut(),(x*scale).round() as i32,(y*scale).round() as i32,(w*scale).round() as i32,(h*scale).round() as i32,SWP_NOACTIVATE|SWP_NOZORDER);ShowWindow(hwnd as HWND,if w==0.||h==0.{SW_HIDE}else{SW_SHOWNA});for handle in std::iter::once(hwnd).chain(overlays){let region=clip.map(|(cx,cy,cw,ch)|CreateRectRgn((cx*scale).round()as i32,(cy*scale).round()as i32,((cx+cw)*scale).round()as i32,((cy+ch)*scale).round()as i32)).unwrap_or(std::ptr::null_mut());if SetWindowRgn(handle as HWND,region,1)==0&&!region.is_null(){DeleteObject(region as _);}}}}).map_err(|_|"播放器位置更新失败")?;
        if let Some(overlay)=&player.overlay{if w==0.||h==0.{overlay.hide().map_err(|_|"弹幕图层隐藏失败")?;}else{overlay.set_bounds(tauri::Rect{position:tauri::LogicalPosition::new(x,y).into(),size:tauri::LogicalSize::new(w,h).into()}).map_err(|_|"弹幕图层位置更新失败")?;overlay.show().map_err(|_|"弹幕图层显示失败")?;}}queue_layers(player)
    }
    fn queue_layers(player:&Player)->Result<(),String>{
        let video=player.hwnd;let overlays=player.overlay_hwnds.clone();
        player.app.run_on_main_thread(move||{let _=crate::native_layers::restack(video,&overlays);}).map_err(|_|"播放器图层更新失败".into())
    }
    pub(crate) fn maintain_layers(app:&AppHandle){
        // Window events execute on the UI thread. Never wait for Player's lock
        // there: a concurrent bounds update can itself be waiting for that UI.
        if RESTACK_PENDING.swap(true,Ordering::AcqRel){return;}
        let app=app.clone();
        tauri::async_runtime::spawn_blocking(move||{
            let layers=slot().lock().ok().and_then(|state|state.as_ref().map(|p|(p.hwnd,p.overlay_hwnds.clone())));
            let Some((video,overlays))=layers else{RESTACK_PENDING.store(false,Ordering::Release);return;};
            if app.run_on_main_thread(move||{let _=crate::native_layers::restack(video,&overlays);RESTACK_PENDING.store(false,Ordering::Release);}).is_err(){RESTACK_PENDING.store(false,Ordering::Release);}
        });
    }
    pub(crate) fn snapshot()->Value{
        let value=slot().lock().map(|guard|guard.as_ref().map(Player::snapshot)).ok().flatten();
        let last=LAST_OBSERVED.get_or_init(||Mutex::new(None));
        if let Some(value)=value{
            if value["frame_ready"]==true{let mut observed=value.clone();observed["active"]=json!(false);observed["observed_at"]=json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64);*last.lock().unwrap()=Some(observed);}
            value
        }else{json!({"active":false,"last_observed":last.lock().unwrap().clone()})}
    }
    pub(crate) fn screenshot(path:&Path)->Result<(),String>{let state=slot().lock().map_err(|_|"原生播放器繁忙")?;let player=state.as_ref().ok_or("原生播放器未打开")?;player.command(&["screenshot-to-file",path.to_str().ok_or("截图路径无效")?,"video"])}
    fn children(parent:usize)->Vec<usize>{let mut children=Vec::new();let mut child=unsafe{GetWindow(parent as HWND,GW_CHILD)};while !child.is_null(){children.push(child as usize);child=unsafe{GetWindow(child,GW_HWNDNEXT)};}children}
    fn create_overlay(app:&AppHandle,x:f64,y:f64,w:f64,h:f64)->Result<Webview,String>{
        let main=app.get_webview("main").ok_or("主界面不可用")?;
        let window=app.get_window("main").ok_or("窗口不可用")?;
        let (tx,rx)=std::sync::mpsc::channel();
        // Reuse the live main controller's environment on its owning UI thread.
        // Creating another environment on replay switches can disconnect from
        // WebView2 (RPC_E_DISCONNECTED); keep the exact profile/browser options.
        main.with_webview(move|platform|{
            let builder=tauri::webview::WebviewBuilder::new("replay-overlay",tauri::WebviewUrl::App("replay-overlay.html".into()))
                .with_environment(platform.environment()).transparent(true).focused(false);
            let result=window.add_child(builder,tauri::LogicalPosition::new(x,y),tauri::LogicalSize::new(w.max(1.),h.max(1.)))
                .map_err(|error|format!("无法创建弹幕图层（{error}）"));
            if let Err(unsent)=tx.send(result){if let Ok(overlay)=unsent.0{let _=overlay.close();}}
        }).map_err(|_|"无法访问主界面图层")?;
        rx.recv_timeout(Duration::from_secs(15)).map_err(|_|"创建弹幕图层超时".to_owned())?
    }
    #[tauri::command]
    pub(crate) async fn native_replay_open(app:AppHandle,state:State<'_,Arc<Backend>>,url:String,bounds:Value,start_time:Option<f64>,paused:Option<bool>,volume:Option<f64>,muted:Option<bool>)->Result<Value,String>{
        let backend=state.inner().clone();
        tauri::async_runtime::spawn_blocking(move||{
            let _operation=backend.read_operation()?;
            let _opening=OPEN.lock().map_err(|_|"原生播放器繁忙")?;
            let runtime=backend.get()?;
            if runtime.request("community_settings",json!({}))?["enabled"]!=true{return Err("社区生态尚未启用".into());}
            let url=h_live_runtime::validated_media_url(&url)?.to_string();
            let (x,y,w,h)=self::bounds(&bounds)?;
            let previous=slot().lock().map_err(|_|"原生播放器繁忙")?.take();drop(previous);
            let api=Api::load(&library(&app)?)?;
            let window=app.get_window("main").ok_or("窗口不可用")?;
            let parent=window.hwnd().map_err(|_|"窗口句柄不可用")?.0 as usize;
            let scale=window.scale_factor().map_err(|_|"窗口缩放不可用")?;
            let (tx,rx)=std::sync::mpsc::channel();
            app.run_on_main_thread(move||{
                let class:Vec<u16>="STATIC\0".encode_utf16().collect();
                let hwnd=unsafe{CreateWindowExW(0,class.as_ptr(),std::ptr::null(),WS_CHILD|if w>0.&&h>0.{WS_VISIBLE}else{0}|WS_CLIPCHILDREN,(x*scale)as i32,(y*scale)as i32,(w*scale)as i32,(h*scale)as i32,parent as HWND,std::ptr::null_mut(),GetModuleHandleW(std::ptr::null()),std::ptr::null())};
                if tx.send(hwnd as usize).is_err()&&!hwnd.is_null(){unsafe{DestroyWindow(hwnd);}}
            }).map_err(|_|"无法创建视频区域")?;
            let hwnd=rx.recv_timeout(Duration::from_secs(5)).map_err(|_|"创建视频区域超时")?;
            if hwnd==0{return Err("无法创建视频区域".into());}
            let handle=unsafe{(api.create)()} as usize;
            if handle==0{let _=app.run_on_main_thread(move||{unsafe{DestroyWindow(hwnd as HWND);}});return Err("无法创建原生播放器".into());}
            let mut player=Player{api,handle,hwnd,overlay:None,overlay_hwnds:Vec::new(),app:app.clone(),id:NEXT.fetch_add(1,Ordering::Relaxed),fullscreen:false,started:Instant::now(),error:Mutex::new(None),performance:Mutex::new(None),file_loaded:AtomicBool::new(false),playback_restarted:AtomicBool::new(false)};
            // Favor lightweight GPU spatial scaling. Keep normal decoding,
            // source timestamps, audio sync and automatic HDR tone mapping.
            for (key,value) in [("terminal","no"),("msg-level","all=no"),("config","no"),("load-scripts","no"),("osc","no"),("input-default-bindings","no"),("input-vo-keyboard","no"),("sub-auto","no"),("audio-file-auto","no"),("ytdl","no"),("vo","gpu-next"),("gpu-api","d3d11"),("gpu-context","d3d11"),("hwdec","auto-safe"),("scale","bilinear"),("cscale","bilinear"),("dscale","bilinear"),("correct-downscaling","no"),("linear-downscaling","no"),("sigmoid-upscaling","no"),("network-timeout","15"),("force-window","yes"),("keep-open","yes"),("idle","yes"),("referrer","https://www.xingtu.cn/")].into_iter().chain(crate::replay_cache::OPTIONS.iter().copied()){
                let name=CString::new(key).unwrap();let value=CString::new(value).unwrap();
                if unsafe{(player.api.option)(player.handle(),name.as_ptr(),value.as_ptr())}<0{return Err(format!("原生播放器参数不可用：{key}"));}
            }
            let wid=CString::new("wid").unwrap();let handle_string=CString::new(hwnd.to_string()).unwrap();
            if unsafe{(player.api.option)(player.handle(),wid.as_ptr(),handle_string.as_ptr())}<0{return Err("无法嵌入原生播放器".into());}
            // Explicitly clear libavformat's proxy option; this does not change
            // the app collector or any system proxy setting.
            let protocols="https,tls,tcp,crypto";
            let options=CString::new(format!("http_proxy=,protocol_whitelist=%{}%{protocols}",protocols.len())).unwrap();
            if unsafe{(player.api.option)(player.handle(),c"stream-lavf-o".as_ptr(),options.as_ptr())}<0{return Err("无法限制原生媒体协议".into());}
            let hls_options=CString::new(crate::replay_cache::HLS_OPTIONS).unwrap();
            if unsafe{(player.api.option)(player.handle(),c"demuxer-lavf-o".as_ptr(),hls_options.as_ptr())}<0{return Err("无法配置录播分段预取".into());}
            if unsafe{(player.api.initialize)(player.handle())}<0{return Err("原生GPU播放器初始化失败".into());}
            player.set_flag("pause",paused.unwrap_or(false))?;
            player.set_number("volume",volume.unwrap_or(1.).clamp(0.,1.)*100.)?;
            player.set_flag("mute",muted.unwrap_or(volume==Some(0.)))?;
            let start=start_time.unwrap_or(0.);if !start.is_finite()||start<0.{return Err("播放起点无效".into());}
            player.command(&["loadfile",&url,"replace","-1",&format!("start={start}")])?;
            let before=children(parent);
            player.overlay=Some(create_overlay(&app,x,y,w,h)?);
            player.overlay_hwnds=children(parent).into_iter().filter(|handle|!before.contains(handle)).collect();
            if player.overlay_hwnds.is_empty(){return Err("弹幕图层窗口未创建".into());}
            if w==0.||h==0.{if let Some(overlay)=&player.overlay{let _=overlay.hide();}}
            place(&player,&bounds)?;
            let result=player.snapshot();*slot().lock().map_err(|_|"原生播放器繁忙")?=Some(player);Ok(result)
        }).await.map_err(|_|"原生播放器启动任务失败".to_owned())?
    }
    #[tauri::command]
    pub(crate) async fn native_replay_status()->Result<Value,String>{tauri::async_runtime::spawn_blocking(||Ok(snapshot())).await.map_err(|_|"原生播放状态任务失败".to_owned())?}
    #[tauri::command]
    pub(crate) async fn native_replay_action(app:AppHandle,action:String,value:Option<f64>)->Result<Value,String>{
        tauri::async_runtime::spawn_blocking(move||{
            let mut state=slot().lock().map_err(|_|"原生播放器繁忙")?;let player=state.as_mut().ok_or("原生播放器未打开")?;
            match action.as_str(){
                "play"=>player.set_flag("pause",false)?,"pause"=>player.set_flag("pause",true)?,
                "seek"=>{let value=value.filter(|v|v.is_finite()&&*v>=0.).ok_or("跳转时间无效")?;player.command(&["seek",&value.to_string(),"absolute+exact"])?;},
                "volume"=>{let value=value.filter(|v|v.is_finite()).ok_or("音量无效")?.clamp(0.,1.);player.set_number("volume",value*100.)?;player.set_flag("mute",value==0.)?;},
                "speed"=>{let value=value.filter(|v|v.is_finite()&&*v>=0.25&&*v<=4.).ok_or("播放速度无效")?;player.set_number("speed",value)?;},
                "mute"=>player.set_flag("mute",value.unwrap_or(1.)!=0.)?,
                "fullscreen"=>{let enabled=value.unwrap_or(1.)!=0.;app.get_window("main").ok_or("窗口不可用")?.set_fullscreen(enabled).map_err(|_|"无法切换全屏")?;player.fullscreen=enabled;queue_layers(player)?;},
                _=>return Err("未知原生播放操作".into())
            }Ok(player.snapshot())
        }).await.map_err(|_|"原生播放操作任务失败".to_owned())?
    }
    #[tauri::command]
    pub(crate) async fn native_replay_bounds(bounds:Value)->Result<(),String>{
        tauri::async_runtime::spawn_blocking(move||{let state=slot().lock().map_err(|_|"原生播放器繁忙")?;if let Some(player)=state.as_ref(){place(player,&bounds)?;}Ok(())}).await.map_err(|_|"原生播放器位置任务失败".to_owned())?
    }
    #[tauri::command]
    pub(crate) async fn native_replay_close()->Result<(),String>{
        tauri::async_runtime::spawn_blocking(||{let _opening=OPEN.lock().map_err(|_|"原生播放器繁忙")?;let previous=slot().lock().map_err(|_|"原生播放器繁忙")?.take();drop(previous);Ok(())}).await.map_err(|_|"原生播放器关闭任务失败".to_owned())?
    }
}
#[cfg(windows)]pub(crate) use platform::*;
