use h_live_server::{auth::Auth, control::Control, http, secrets};
use salvo::prelude::*;
use std::{env, path::PathBuf, sync::Arc, time::Duration};

fn main() {
    if let Err(e) = run() {
        eprintln!("H-live server: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::umask(0o077);
    }
    let args: Vec<_> = env::args().skip(1).collect();
    let secret_dir = PathBuf::from(
        env::var("H_LIVE_SECRETS_DIR").unwrap_or_else(|_| "/run/h-live-secrets".into()),
    );
    match args.first().map(String::as_str).unwrap_or("serve") {
        "--version" | "version" => {
            println!("h-live-server {}", h_live_server::VERSION);
            return Ok(());
        }
        "init-secrets" => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or(secret_dir);
            let n = secrets::init(&dir)?;
            println!("Initialized {n} secret files (existing files preserved).");
            return Ok(());
        }
        "healthcheck" => {
            let addr = env::var("H_LIVE_HEALTH_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect_timeout(
                &addr.parse().map_err(|_| "健康检查地址无效")?,
                Duration::from_secs(3),
            )
            .map_err(|_| "服务不可达")?;
            s.set_read_timeout(Some(Duration::from_secs(3)))
                .map_err(|_| "超时设置失败")?;
            s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .map_err(|_| "检查失败")?;
            let mut b = [0; 256];
            let n = s.read(&mut b).map_err(|_| "检查失败")?;
            if !b[..n].starts_with(b"HTTP/1.1 200") {
                return Err("服务未就绪".into());
            }
            return Ok(());
        }
        "serve" => {}
        _ => return Err("命令: serve | init-secrets [directory] | healthcheck | --version".into()),
    }
    let data =
        PathBuf::from(env::var("H_LIVE_DATA_DIR").unwrap_or_else(|_| "/var/lib/h-live".into()));
    let ui = PathBuf::from(env::var("H_LIVE_UI_DIR").unwrap_or_else(|_| "/opt/h-live/ui".into()))
        .canonicalize()
        .map_err(|_| "管理界面不存在，请构建 server UI")?;
    if !ui.join("index.html").is_file() {
        return Err("管理界面缺少 index.html".into());
    }
    let bind = env::var("H_LIVE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let origin =
        env::var("H_LIVE_PUBLIC_ORIGIN").unwrap_or_else(|_| "http://localhost:8080".into());
    let auth = Auth::new(secrets::read_key(&secret_dir.join("admin-token"))?, &origin)?;
    let protector = Arc::new(secrets::ServerProtector::load(
        &secret_dir.join("master.key"),
    )?);
    // Runtime construction/drop must occur outside the async HTTP executor.
    let control = Control::open(data, protector)?;
    let app = http::App::new(control.clone(), auth);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()
        .map_err(|_| "HTTP 运行时初始化失败")?;
    let result = rt.block_on(async {
        let acceptor = TcpListener::new(bind.clone())
            .try_bind()
            .await
            .map_err(|_| "HTTP 监听失败")?;
        let server = Server::new(acceptor);
        let handle = server.handle();
        let signal = tokio::spawn(async move {
            shutdown_signal().await;
            handle.stop_graceful(Duration::from_secs(20));
        });
        println!(
            "H-live server {} listening on {bind}",
            h_live_server::VERSION
        );
        server.serve(http::router(app, ui)).await;
        signal.abort();
        Ok::<_, String>(())
    });
    let stopped = rt.block_on(control.shutdown());
    drop(control);
    drop(rt);
    result?;
    stopped?;
    println!("All monitoring workers stopped and archives flushed.");
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM registration");
        tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
